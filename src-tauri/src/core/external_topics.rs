#[derive(Debug, Clone, Copy)]
pub struct ExternalTopicDefinition {
    pub key: &'static str,
    pub label: &'static str,
    pub category: &'static str,
    pub total_count: u32,
    pub osv_ecosystem: Option<&'static str>,
    pub default_watched: bool,
}

const TOPICS: &[ExternalTopicDefinition] = &[
    ExternalTopicDefinition {
        key: "npm",
        label: "npm",
        category: "Packages",
        total_count: 217_504,
        osv_ecosystem: Some("npm"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "pip",
        label: "PyPI",
        category: "Packages",
        total_count: 18_697,
        osv_ecosystem: Some("PyPI"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "go",
        label: "Go",
        category: "Packages",
        total_count: 6_580,
        osv_ecosystem: Some("Go"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "maven",
        label: "Maven",
        category: "Packages",
        total_count: 6_325,
        osv_ecosystem: Some("Maven"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "composer",
        label: "Packagist",
        category: "Packages",
        total_count: 6_082,
        osv_ecosystem: Some("Packagist"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "rust",
        label: "crates.io",
        category: "Packages",
        total_count: 2_232,
        osv_ecosystem: Some("crates.io"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "rubygems",
        label: "RubyGems",
        category: "Packages",
        total_count: 1_938,
        osv_ecosystem: Some("RubyGems"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "nuget",
        label: "NuGet",
        category: "Packages",
        total_count: 1_659,
        osv_ecosystem: Some("NuGet"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "cran",
        label: "CRAN",
        category: "Packages",
        total_count: 14,
        osv_ecosystem: Some("CRAN"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "pub",
        label: "Pub",
        category: "Packages",
        total_count: 11,
        osv_ecosystem: Some("Pub"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "swift",
        label: "SwiftURL",
        category: "Packages",
        total_count: 50,
        osv_ecosystem: Some("SwiftURL"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "hackage",
        label: "Hackage",
        category: "Packages",
        total_count: 30,
        osv_ecosystem: Some("Hackage"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "hex",
        label: "Hex",
        category: "Packages",
        total_count: 57,
        osv_ecosystem: Some("Hex"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "opam",
        label: "opam",
        category: "Packages",
        total_count: 12,
        osv_ecosystem: Some("opam"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "julia",
        label: "Julia",
        category: "Packages",
        total_count: 513,
        osv_ecosystem: Some("Julia"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "ghc",
        label: "GHC",
        category: "Packages",
        total_count: 3,
        osv_ecosystem: Some("GHC"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "actions",
        label: "GitHub Actions",
        category: "Dev tooling",
        total_count: 49,
        osv_ecosystem: Some("GitHub Actions"),
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "github_enterprise_server",
        label: "GitHub Enterprise Server",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "github_actions_runner",
        label: "GitHub Actions Runner",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: true,
    },
    ExternalTopicDefinition {
        key: "gitlab",
        label: "GitLab",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "jenkins",
        label: "Jenkins",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "kubernetes",
        label: "Kubernetes",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "docker",
        label: "Docker",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "nginx",
        label: "nginx",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "postgres",
        label: "PostgreSQL",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "openssl",
        label: "OpenSSL",
        category: "Products and infrastructure",
        total_count: 0,
        osv_ecosystem: None,
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "vscode",
        label: "VSCode",
        category: "Dev tooling",
        total_count: 18,
        osv_ecosystem: Some("VSCode"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "oss-fuzz",
        label: "OSS-Fuzz",
        category: "Security research",
        total_count: 3_835,
        osv_ecosystem: Some("OSS-Fuzz"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "git",
        label: "GIT",
        category: "Security research",
        total_count: 81_483,
        osv_ecosystem: Some("GIT"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "ubuntu",
        label: "Ubuntu",
        category: "Linux and distros",
        total_count: 52_357,
        osv_ecosystem: Some("Ubuntu"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "debian",
        label: "Debian",
        category: "Linux and distros",
        total_count: 54_522,
        osv_ecosystem: Some("Debian"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "red hat",
        label: "Red Hat",
        category: "Linux and distros",
        total_count: 19_421,
        osv_ecosystem: Some("Red Hat"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "suse",
        label: "SUSE",
        category: "Linux and distros",
        total_count: 20_489,
        osv_ecosystem: Some("SUSE"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "opensuse",
        label: "openSUSE",
        category: "Linux and distros",
        total_count: 12_558,
        osv_ecosystem: Some("openSUSE"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "alpine",
        label: "Alpine",
        category: "Linux and distros",
        total_count: 4_049,
        osv_ecosystem: Some("Alpine"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "almalinux",
        label: "AlmaLinux",
        category: "Linux and distros",
        total_count: 4_656,
        osv_ecosystem: Some("AlmaLinux"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "rocky linux",
        label: "Rocky Linux",
        category: "Linux and distros",
        total_count: 2_948,
        osv_ecosystem: Some("Rocky Linux"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "mageia",
        label: "Mageia",
        category: "Linux and distros",
        total_count: 5_877,
        osv_ecosystem: Some("Mageia"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "openeuler",
        label: "openEuler",
        category: "Linux and distros",
        total_count: 6_386,
        osv_ecosystem: Some("openEuler"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "linux",
        label: "Linux",
        category: "Linux and distros",
        total_count: 15_361,
        osv_ecosystem: Some("Linux"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "android",
        label: "Android",
        category: "Platforms",
        total_count: 3_261,
        osv_ecosystem: Some("Android"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "azure linux",
        label: "Azure Linux",
        category: "Container images",
        total_count: 12_016,
        osv_ecosystem: Some("Azure Linux"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "chainguard",
        label: "Chainguard",
        category: "Container images",
        total_count: 5_735,
        osv_ecosystem: Some("Chainguard"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "wolfi",
        label: "Wolfi",
        category: "Container images",
        total_count: 3_685,
        osv_ecosystem: Some("Wolfi"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "bitnami",
        label: "Bitnami",
        category: "Container images",
        total_count: 6_983,
        osv_ecosystem: Some("Bitnami"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "bellsoft hardened containers",
        label: "BellSoft Hardened Containers",
        category: "Container images",
        total_count: 432,
        osv_ecosystem: Some("BellSoft Hardened Containers"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "minimos",
        label: "MinimOS",
        category: "Container images",
        total_count: 26_878,
        osv_ecosystem: Some("MinimOS"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "alpaquita",
        label: "Alpaquita",
        category: "Container images",
        total_count: 8_853,
        osv_ecosystem: Some("Alpaquita"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "cleanstart",
        label: "CleanStart",
        category: "Container images",
        total_count: 791,
        osv_ecosystem: Some("CleanStart"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "root",
        label: "Root",
        category: "Container images",
        total_count: 11_970,
        osv_ecosystem: Some("Root"),
        default_watched: false,
    },
    ExternalTopicDefinition {
        key: "echo",
        label: "Echo",
        category: "Container images",
        total_count: 3_188,
        osv_ecosystem: Some("Echo"),
        default_watched: false,
    },
];

pub fn topic_definitions() -> &'static [ExternalTopicDefinition] {
    TOPICS
}

pub fn default_topic_keys() -> Vec<String> {
    TOPICS
        .iter()
        .filter(|topic| topic.default_watched)
        .map(|topic| topic.key.to_string())
        .collect()
}

pub fn normalize_topic_key(value: &str) -> Option<String> {
    let normalized = normalize_external_ecosystem(value)?;
    TOPICS
        .iter()
        .any(|topic| topic.key == normalized)
        .then_some(normalized)
}

pub fn normalize_external_ecosystem(value: &str) -> Option<String> {
    let normalized = match value.trim() {
        "npm" | "NPM" => "npm",
        "PyPI" | "pip" | "PIP" => "pip",
        "RubyGems" | "rubygems" | "gem" => "rubygems",
        "Maven" | "maven" => "maven",
        "NuGet" | "nuget" => "nuget",
        "Composer" | "composer" | "Packagist" => "composer",
        "Go" | "go" => "go",
        "crates.io" | "Rust" | "rust" => "rust",
        "actions" | "GitHub Actions" => "actions",
        "github_enterprise_server" | "github enterprise server" | "ghes" => {
            "github_enterprise_server"
        }
        "github_actions_runner" | "github actions runner" | "actions runner" => {
            "github_actions_runner"
        }
        "gitlab" | "GitLab" => "gitlab",
        "jenkins" | "Jenkins" => "jenkins",
        "kubernetes" | "Kubernetes" | "k8s" => "kubernetes",
        "docker" | "Docker" => "docker",
        "nginx" | "NGINX" => "nginx",
        "postgres" | "postgresql" | "PostgreSQL" => "postgres",
        "openssl" | "OpenSSL" => "openssl",
        "Pub" | "pub" | "dart-pub" => "pub",
        "SwiftURL" | "swift" | "Swift" => "swift",
        "OSS-Fuzz" | "oss-fuzz" => "oss-fuzz",
        "GIT" | "git" => "git",
        "VSCode" | "vscode" => "vscode",
        "AlmaLinux" | "almalinux" => "almalinux",
        "Alpaquita" | "alpaquita" => "alpaquita",
        "Alpine" | "alpine" => "alpine",
        "Android" | "android" => "android",
        "Azure Linux" | "azure linux" => "azure linux",
        "BellSoft Hardened Containers" | "bellsoft hardened containers" => {
            "bellsoft hardened containers"
        }
        "Bitnami" | "bitnami" => "bitnami",
        "Chainguard" | "chainguard" => "chainguard",
        "CleanStart" | "cleanstart" => "cleanstart",
        "CRAN" | "cran" => "cran",
        "Debian" | "debian" => "debian",
        "Echo" | "echo" => "echo",
        "GHC" | "ghc" => "ghc",
        "Hackage" | "hackage" => "hackage",
        "Hex" | "hex" => "hex",
        "Julia" | "julia" => "julia",
        "Linux" | "linux" => "linux",
        "Mageia" | "mageia" => "mageia",
        "MinimOS" | "minimos" => "minimos",
        "opam" => "opam",
        "openEuler" | "openeuler" => "openeuler",
        "openSUSE" | "opensuse" => "opensuse",
        "Red Hat" | "red hat" => "red hat",
        "Rocky Linux" | "rocky linux" => "rocky linux",
        "Root" | "root" => "root",
        "SUSE" | "suse" => "suse",
        "Ubuntu" | "ubuntu" => "ubuntu",
        "Wolfi" | "wolfi" => "wolfi",
        _ => return None,
    };

    Some(normalized.to_string())
}

pub fn product_key(vendor: &str, product: &str) -> String {
    format!(
        "{}:{}",
        normalize_product_part(vendor),
        normalize_product_part(product)
    )
}

pub fn product_key_to_topic(value: &str) -> Option<String> {
    let topic = match value {
        "github:enterprise_server" => "github_enterprise_server",
        "github:actions_runner" => "github_actions_runner",
        "gitlab:gitlab" => "gitlab",
        "jenkins:jenkins" => "jenkins",
        "kubernetes:kubernetes" => "kubernetes",
        "docker:docker" => "docker",
        "nginx:nginx" => "nginx",
        "postgresql:postgresql" | "postgres:postgres" => "postgres",
        "openssl:openssl" => "openssl",
        _ => return None,
    };
    Some(topic.to_string())
}

fn normalize_product_part(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}
