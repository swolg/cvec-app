use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{Connection, Statement};

use super::{normalize_local_package_type, Database, LATEST_COMPLETED_SCAN_FOR_REPO};
use crate::core::delta::purl_without_version;
use crate::core::external_topics::normalize_external_ecosystem;
use crate::core::models::{
    AdvisoryPackageImpact, DataRevisions, ExternalIntelLocalPackage, ExternalIntelMatchKind,
    ScannedPackageIdentity,
};
use crate::core::scanner::{normalize_python_package_name, package_identity};
use crate::Error;

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub(in crate::core) struct AdvisoryPackageKey {
    ecosystem: String,
    name: String,
}

impl AdvisoryPackageKey {
    pub(super) fn new(
        ecosystem: Option<&str>,
        name: Option<&str>,
        purl: Option<&str>,
    ) -> Option<Self> {
        let (purl_type, purl_path) = purl
            .and_then(purl_without_version)
            .and_then(|(path, _)| path.split_once('/'))
            .unzip();
        let purl_ecosystem = purl_type.and_then(package_ecosystem);
        let purl_path = purl_path.filter(|_| purl_ecosystem.is_some());
        let ecosystem = purl_ecosystem.or_else(|| ecosystem.and_then(package_ecosystem))?;
        // Prefer the registry coordinates from the PURL for namespaced packages.
        let name = match ecosystem.as_str() {
            "maven" => purl_path
                .and_then(|path| path.rsplit_once('/'))
                .map(|(group, artifact)| format!("{group}:{artifact}"))
                .or_else(|| name.filter(|name| name.contains(':')).map(str::to_owned))?,
            "npm" => {
                let name = purl_path.or(name)?;
                match name.strip_prefix("%40") {
                    Some(scoped) => format!("@{scoped}").to_ascii_lowercase(),
                    None => name.to_ascii_lowercase(),
                }
            }
            "pip" => normalize_python_package_name(purl_path.or(name)?)?,
            "nuget" => purl_path.or(name)?.to_ascii_lowercase(),
            _ => purl_path.or(name)?.to_owned(),
        };
        (!name.trim().is_empty()).then_some(Self { ecosystem, name })
    }
}

fn package_ecosystem(value: &str) -> Option<String> {
    normalize_local_package_type(value)
        .map(str::to_owned)
        .or_else(|| normalize_external_ecosystem(value))
}

pub(in crate::core) fn advisory_package_keys(
    ecosystem: Option<&str>,
    name: Option<&str>,
    affected: &[AdvisoryPackageImpact],
) -> HashSet<AdvisoryPackageKey> {
    let mut keys: HashSet<_> = affected
        .iter()
        .filter_map(|package| {
            AdvisoryPackageKey::new(
                package.ecosystem.as_deref(),
                package.package_name.as_deref(),
                package.package_purl.as_deref(),
            )
        })
        .collect();
    if keys.is_empty() {
        keys.extend(AdvisoryPackageKey::new(ecosystem, name, None));
    }
    keys
}

#[derive(Debug, Default)]
pub(in crate::core) struct LocalPackageIndex {
    packages: HashMap<AdvisoryPackageKey, PackageMatches>,
    pub(in crate::core) ecosystem_repo_counts: HashMap<String, usize>,
    pub(in crate::core) product_repo_counts: HashMap<String, usize>,
    vendor_repo_counts: HashMap<String, usize>,
}

#[derive(Debug, Default)]
struct PackageMatches {
    repositories: HashSet<i64>,
    packages: Vec<ExternalIntelLocalPackage>,
}

impl LocalPackageIndex {
    #[cfg(test)]
    pub(in crate::core) fn add_test_package(&mut self, ecosystem: &str, name: &str, repo_id: i64) {
        let key = AdvisoryPackageKey::new(Some(ecosystem), Some(name), None).unwrap();
        self.packages
            .entry(key)
            .or_default()
            .repositories
            .insert(repo_id);
    }

    pub(in crate::core) fn match_advisory(
        &self,
        packages: &HashSet<AdvisoryPackageKey>,
        product_keys: &[String],
        ecosystem: Option<&str>,
    ) -> (Option<ExternalIntelMatchKind>, usize) {
        let mut repositories = packages
            .iter()
            .filter_map(|key| self.packages.get(key).map(|matches| &matches.repositories));
        if let Some(first) = repositories.next() {
            let count = match repositories.next() {
                Some(second) => first
                    .iter()
                    .chain(second)
                    .chain(repositories.flatten())
                    .collect::<HashSet<_>>()
                    .len(),
                None => first.len(),
            };
            return (Some(ExternalIntelMatchKind::PackageName), count);
        }
        for key in product_keys {
            if let Some(&count) = self.product_repo_counts.get(key) {
                return (Some(ExternalIntelMatchKind::Product), count);
            }
        }
        for key in product_keys {
            if let Some((vendor, _)) = key.split_once(':') {
                if let Some(&count) = self.vendor_repo_counts.get(vendor) {
                    return (Some(ExternalIntelMatchKind::Vendor), count);
                }
            }
        }
        if let Some(&count) = ecosystem.and_then(|key| self.ecosystem_repo_counts.get(key)) {
            return (Some(ExternalIntelMatchKind::Ecosystem), count);
        }
        (None, 0)
    }

    pub(super) fn load(
        connection: &Connection,
        wanted: &HashSet<AdvisoryPackageKey>,
    ) -> Result<Self, Error> {
        let mut index = Self::default();
        let mut ecosystem_repos: HashMap<String, HashSet<i64>> = HashMap::new();
        let mut seen = HashSet::new();
        let mut packages = latest_packages(connection)?;
        let rows = packages.query_map([], |row| {
            let repo_id: i64 = row.get(0)?;
            let name = row.get_ref(2)?.as_str()?;
            let package_type = row.get_ref(4)?.as_str_or_null()?;
            let package_url = row.get_ref(5)?.as_str_or_null()?;
            let Some(key) = AdvisoryPackageKey::new(package_type, Some(name), package_url) else {
                return Ok(());
            };
            if let Some(repos) = ecosystem_repos.get_mut(&key.ecosystem) {
                repos.insert(repo_id);
            } else {
                ecosystem_repos.insert(key.ecosystem.clone(), HashSet::from([repo_id]));
            }
            if !wanted.contains(&key) {
                return Ok(());
            }
            let version = row.get_ref(3)?.as_str_or_null()?;
            let identity = (
                repo_id,
                package_identity(package_url, name, version, package_type),
            );
            // The first arm of UNION ALL returns the inventory rows. If a
            // finding also reports that version, keep the inventory's row.
            if seen.insert(identity) {
                let matches = index.packages.entry(key.clone()).or_default();
                matches.repositories.insert(repo_id);
                matches.packages.push(ExternalIntelLocalPackage {
                    repo_id,
                    repo_name: row.get(1)?,
                    ecosystem: key.ecosystem,
                    package_name: key.name,
                    affected_version_ranges: Vec::new(),
                    package: ScannedPackageIdentity {
                        name: name.to_owned(),
                        version: version.map(str::to_owned),
                        package_type: package_type.map(str::to_owned),
                        package_url: package_url.map(str::to_owned),
                    },
                });
            }
            Ok(())
        })?;
        for row in rows {
            row?;
        }
        index.ecosystem_repo_counts = ecosystem_repos
            .into_iter()
            .map(|(ecosystem, repos)| (ecosystem, repos.len()))
            .collect();

        let mut assets = connection.prepare(
            "SELECT DISTINCT r.id, a.vendor, a.product FROM repo_assets a
             JOIN repos r ON r.id = a.repo_id WHERE r.is_active = 1",
        )?;
        let mut rows = assets.query([])?;
        let mut repo_vendors = HashSet::new();
        while let Some(row) = rows.next()? {
            let repo_id: i64 = row.get(0)?;
            let vendor: String = row.get(1)?;
            let product: String = row.get(2)?;
            *index
                .product_repo_counts
                .entry(format!("{vendor}:{product}"))
                .or_default() += 1;
            if repo_vendors.insert((repo_id, vendor.clone())) {
                *index.vendor_repo_counts.entry(vendor).or_default() += 1;
            }
        }
        Ok(index)
    }

    pub(super) fn local_packages(
        &self,
        keys: &HashSet<AdvisoryPackageKey>,
        affected: &[AdvisoryPackageImpact],
    ) -> Vec<ExternalIntelLocalPackage> {
        let mut ranges: HashMap<_, BTreeSet<_>> = HashMap::new();
        for impact in affected {
            let key = AdvisoryPackageKey::new(
                impact.ecosystem.as_deref(),
                impact.package_name.as_deref(),
                impact.package_purl.as_deref(),
            );
            if let (Some(key), Some(range)) = (key, impact.vulnerable_version_range.as_deref()) {
                ranges.entry(key).or_default().insert(range);
            }
        }
        let mut packages = Vec::new();
        for key in keys {
            let Some(matches) = self.packages.get(key) else {
                continue;
            };
            let affected_version_ranges: Vec<_> = ranges
                .get(key)
                .into_iter()
                .flatten()
                .map(|range| (*range).to_owned())
                .collect();
            packages.extend(matches.packages.iter().cloned().map(|mut item| {
                item.affected_version_ranges = affected_version_ranges.clone();
                item
            }));
        }
        packages.sort_unstable_by(|a, b| {
            a.repo_name
                .cmp(&b.repo_name)
                .then_with(|| a.repo_id.cmp(&b.repo_id))
                .then_with(|| a.package_name.cmp(&b.package_name))
                .then_with(|| a.package.version.cmp(&b.package.version))
                .then_with(|| a.package.package_type.cmp(&b.package.package_type))
                .then_with(|| a.package.package_url.cmp(&b.package.package_url))
        });
        packages
    }
}

#[derive(Debug)]
pub(super) struct CachedIntelPackages {
    intel_revision: u64,
    inventory_revision: u64,
    index: Arc<LocalPackageIndex>,
}

impl Database {
    pub(in crate::core) fn latest_local_package_index(
        &self,
        wanted: &HashSet<AdvisoryPackageKey>,
    ) -> Result<LocalPackageIndex, Error> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        LocalPackageIndex::load(&transaction, wanted)
    }

    pub(super) fn cached_intel_packages(
        &self,
        revisions: &DataRevisions,
    ) -> Option<Arc<LocalPackageIndex>> {
        let cache = self.intel_packages.lock().ok()?;
        cache
            .as_ref()
            .filter(|entry| {
                entry.intel_revision == revisions.intel
                    && entry.inventory_revision == revisions.inventory
            })
            .map(|entry| Arc::clone(&entry.index))
    }

    pub(super) fn intel_feed_packages(
        &self,
        connection: &Connection,
        revisions: &DataRevisions,
        keys: &HashMap<i64, HashSet<AdvisoryPackageKey>>,
    ) -> Result<Arc<LocalPackageIndex>, Error> {
        if let Some(index) = self.cached_intel_packages(revisions) {
            return Ok(index);
        }
        let wanted = keys.values().flatten().cloned().collect();
        let index = Arc::new(LocalPackageIndex::load(connection, &wanted)?);
        if let Ok(mut cache) = self.intel_packages.lock() {
            // An older read may finish after a scan or an advisory refresh.
            if cache.as_ref().is_none_or(|entry| {
                entry.intel_revision <= revisions.intel
                    && entry.inventory_revision <= revisions.inventory
            }) {
                *cache = Some(CachedIntelPackages {
                    intel_revision: revisions.intel,
                    inventory_revision: revisions.inventory,
                    index: Arc::clone(&index),
                });
            }
        }
        Ok(index)
    }
}

fn latest_packages(connection: &Connection) -> rusqlite::Result<Statement<'_>> {
    connection.prepare(&format!(
        "WITH active_runs AS MATERIALIZED (
             SELECT r.id, r.name, ({LATEST_COMPLETED_SCAN_FOR_REPO}) scan_id
             FROM repos r WHERE r.is_active = 1
         )
         SELECT r.id, r.name, p.name, p.version, p.package_type, p.package_url
         FROM active_runs r CROSS JOIN scan_packages p
         WHERE p.repo_id = r.id AND p.scan_run_id = r.scan_id
         UNION ALL
         SELECT r.id, r.name, v.package_name, v.package_version, v.package_type, v.package_url
         FROM active_runs r CROSS JOIN vulnerabilities v
         WHERE v.repo_id = r.id AND v.scan_run_id = r.scan_id"
    ))
}
