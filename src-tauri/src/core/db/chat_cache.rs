use super::*;

impl Database {
    pub fn companion_cached_citations(
        &self,
        thread_id: i64,
        item_ids: &[String],
        connection_id: &str,
        provider_thread_id: &str,
    ) -> Result<
        std::collections::HashMap<String, Vec<super::super::companion::CompanionCitation>>,
        Error,
    > {
        let connection = self.connect()?;
        let mut statement=connection.prepare("SELECT item_key,json_extract(item_json,'$.citations') FROM companion_timeline_items WHERE thread_id=?1 AND provider_segment_id=(SELECT id FROM companion_provider_segments WHERE thread_id=?1 AND connection_id=?3 AND provider_thread_id=?4 ORDER BY ordinal DESC,id DESC LIMIT 1) AND item_key IN (SELECT value FROM json_each(?2)) AND json_array_length(item_json,'$.citations')>0")?;
        let mut citations = std::collections::HashMap::new();
        for chunk in item_ids.chunks(200) {
            for row in statement.query_map(
                params![
                    thread_id,
                    serde_json::to_string(chunk)?,
                    connection_id,
                    provider_thread_id
                ],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )? {
                let (id, json) = row?;
                citations.insert(id, serde_json::from_str(&json)?);
            }
        }
        Ok(citations)
    }
    pub fn cached_companion_thread_page(
        &self,
        thread_id: i64,
        cursor: Option<&str>,
    ) -> Result<CompanionCachedThreadSnapshot, Error> {
        let boundary = if let Some(cursor) = cursor {
            if cursor.len() > 256 {
                return Err(Error::App("invalid history cursor".into()));
            }
            let parts = cursor.split(':').collect::<Vec<_>>();
            if parts.len() != 4
                || parts[0] != "h1"
                || parts[1].parse::<i64>().ok() != Some(thread_id)
            {
                return Err(Error::App("invalid history cursor".into()));
            }
            let sequence = parts[2]
                .parse::<i64>()
                .map_err(|_| Error::App("invalid history cursor".into()))?;
            let id = parts[3]
                .parse::<i64>()
                .map_err(|_| Error::App("invalid history cursor".into()))?;
            if sequence <= 0 || id <= 0 {
                return Err(Error::App("invalid history cursor".into()));
            }
            Some((sequence, id))
        } else {
            None
        };
        let thread = self.companion_thread(thread_id)?;
        let mut connection = self.connect()?;
        let tx = connection.transaction()?;
        let all_segments = {
            let mut statement=tx.prepare("SELECT id,thread_id,ordinal,provider,connection_id,provider_thread_id,initial_model_id,last_model_id,status,created_at,updated_at,ended_at FROM companion_provider_segments WHERE thread_id=?1 ORDER BY ordinal,id")?;
            let rows = statement.query_map([thread_id], companion_provider_segment_from_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let latest_ordinal = all_segments.last().map(|segment| segment.ordinal);
        let provider_history_available = all_segments.last().is_none_or(|segment| {
            segment.status != CompanionProviderSegmentStatus::HistoryUnavailable
        });
        let mut statement=tx.prepare("SELECT timeline.id,timeline.sequence,timeline.provider_segment_id,segment.ordinal,timeline.item_json FROM companion_timeline_items timeline JOIN companion_provider_segments segment ON segment.id=timeline.provider_segment_id WHERE timeline.thread_id=?1 AND (?2 IS NULL OR timeline.sequence<?2 OR (timeline.sequence=?2 AND timeline.id<?3)) ORDER BY timeline.sequence DESC,timeline.id DESC LIMIT 200")?;
        let mut rows = statement.query(params![
            thread_id,
            boundary.map(|value| value.0),
            boundary.map(|value| value.1)
        ])?;
        let mut items = Vec::new();
        let mut ordinals = std::collections::BTreeMap::new();
        let mut selected_ordinals = std::collections::BTreeSet::new();
        if let Some(ordinal) = latest_ordinal {
            selected_ordinals.insert(ordinal);
        }
        let mut oldest = None;
        let mut bytes = 0;
        let mut streaming_item_ids = Vec::new();
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let sequence: i64 = row.get(1)?;
            let segment: i64 = row.get(2)?;
            let ordinal: u32 = row.get(3)?;
            let json: String = row.get(4)?;
            // A single existing large message remains readable. Other pages stay
            // within 512 KiB, so pagination cannot materialize the entire history.
            if !items.is_empty() && bytes + json.len() > 512 * 1024 {
                break;
            }
            bytes += json.len();
            let mut item: CompanionTimelineItem = serde_json::from_str(&json)?;
            if !item.id.starts_with("local:") {
                item.id = format!("segment:{segment}:{}", item.id);
            }
            if item.status == CompanionTimelineItemStatus::Streaming {
                streaming_item_ids.push(item.id.clone());
                item.status = CompanionTimelineItemStatus::Interrupted;
            }
            if item.kind == CompanionTimelineItemKind::Error
                && item.error.as_ref().is_some_and(|error| {
                    matches!(error.code.as_str(), "codex_warning" | "action_denied")
                })
            {
                item.kind = CompanionTimelineItemKind::Warning;
                item.status = CompanionTimelineItemStatus::Completed;
            }
            selected_ordinals.insert(ordinal);
            ordinals.insert(item.id.clone(), ordinal);
            items.push(item);
            oldest = Some((sequence, id));
        }
        drop(rows);
        drop(statement);
        let older_cursor = if let Some((sequence, id)) = oldest {
            let more=tx.query_row("SELECT EXISTS(SELECT 1 FROM companion_timeline_items WHERE thread_id=?1 AND (sequence<?2 OR (sequence=?2 AND id<?3)))",params![thread_id,sequence,id],|r|r.get::<_,bool>(0))?;
            more.then(|| format!("h1:{thread_id}:{sequence}:{id}"))
        } else {
            None
        };
        items.reverse();
        let segments = all_segments
            .into_iter()
            .filter(|segment| selected_ordinals.contains(&segment.ordinal))
            .collect();
        let event_watermarks = {
            let mut statement=tx.prepare("SELECT provider_segment_id,connection_id,epoch,sequence FROM companion_route_sequences WHERE thread_id=?1")?;
            let rows = statement.query_map([thread_id], |row| {
                Ok((
                    format!(
                        "{}:{}:{}",
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?
                    ),
                    row.get::<_, i64>(3)?.max(0) as u64,
                ))
            })?;
            rows.collect::<Result<std::collections::BTreeMap<_, _>, _>>()?
        };
        tx.commit()?;
        Ok(CompanionCachedThreadSnapshot {
            event_watermarks,
            streaming_item_ids,
            schema_version: COMPANION_SCHEMA_VERSION,
            thread,
            segments,
            items,
            provider_history_available,
            older_cursor,
            provider_ordinals: ordinals,
        })
    }

    /// All deduplication claims and item/status changes share one transaction.
    /// Deltas for a route are appended in memory, then the item is written once.
    pub fn cache_companion_events(
        &self,
        events: &[CompanionEventEnvelope],
    ) -> Result<Vec<CompanionEventEnvelope>, Error> {
        let mut connection = self.connect()?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut pending = Vec::<(i64, i64, String, CompanionTimelineItem)>::new();
        let mut positions = std::collections::HashMap::<(i64, i64, String), usize>::new();
        let mut touched = std::collections::BTreeSet::new();
        let mut accepted = Vec::with_capacity(events.len());
        for event in events {
            let route = match event.thread_id {
                Some(thread) => self
                    .companion_provider_segment_for_event_on(
                        &tx,
                        thread,
                        event.connection_id.as_deref(),
                        event.provider_thread_id.as_deref(),
                    )?
                    .map(|segment| (thread, segment.id)),
                None => None,
            };
            if let Some((_, segment)) = route {
                touched.insert(segment);
            }
            if event.kind == CompanionEventKind::ItemDelta {
                let Some((thread, segment)) = route else {
                    accepted.push(event.clone());
                    continue;
                };
                let event_key = companion_event_cache_key(event)?;
                if !self.claim_companion_cached_event_on(&tx, segment, &event_key)? {
                    continue;
                }
                let key = event
                    .payload
                    .get("itemId")
                    .and_then(serde_json::Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                    .unwrap_or("assistant");
                let delta = event
                    .payload
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let route_key = (thread, segment, key.to_string());
                let index = if let Some(index) = positions.get(&route_key) {
                    *index
                } else {
                    let mut item = self
                        .cached_companion_timeline_item_on(&tx, thread, segment, key)?
                        .unwrap_or_else(|| CompanionTimelineItem {
                            id: key.into(),
                            kind: CompanionTimelineItemKind::AssistantMessage,
                            status: CompanionTimelineItemStatus::Streaming,
                            text: String::new(),
                            activity: None,
                            citations: Vec::new(),
                            error: None,
                            model_id: event.model_id.clone(),
                            created_at: now(),
                        });
                    if item.model_id.is_none() {
                        item.model_id.clone_from(&event.model_id);
                    }
                    let index = pending.len();
                    positions.insert(route_key, index);
                    pending.push((thread, segment, key.into(), item));
                    index
                };
                let item = &mut pending[index].3;
                if item.text.len().saturating_add(delta.len()) > 2_000_000 {
                    return Err(Error::App(
                        "Conversation response exceeded the supported size".into(),
                    ));
                }
                item.text.push_str(delta);
                item.status = CompanionTimelineItemStatus::Streaming;
                let mut forwarded = event.clone();
                forwarded.provider_segment_id = Some(segment);
                accepted.push(forwarded);
            } else {
                for (thread, segment, _, item) in pending.drain(..) {
                    self.upsert_companion_timeline_item_on(&tx, thread, segment, &item)?;
                }
                positions.clear();
                if self.cache_companion_event_on(&tx, event)? {
                    let mut forwarded = event.clone();
                    forwarded.provider_segment_id = route.map(|(_, segment)| segment);
                    accepted.push(forwarded);
                }
            }
        }
        for (thread, segment, _, item) in pending {
            self.upsert_companion_timeline_item_on(&tx, thread, segment, &item)?;
        }
        for segment in touched {
            tx.execute("DELETE FROM companion_cached_events WHERE provider_segment_id=?1 AND id NOT IN (SELECT id FROM companion_cached_events WHERE provider_segment_id=?1 ORDER BY id DESC LIMIT 4096)",[segment])?;
        }
        let mut watermarks = std::collections::HashMap::new();
        for event in &accepted {
            if let (Some(thread), Some(segment), Some(connection)) = (
                event.thread_id,
                event.provider_segment_id,
                event.connection_id.as_deref(),
            ) {
                let sequence = i64::try_from(event.sequence)
                    .map_err(|_| Error::App("Invalid conversation event sequence".into()))?;
                watermarks
                    .entry((
                        thread,
                        segment,
                        connection,
                        event.cache_epoch.as_deref().unwrap_or(""),
                    ))
                    .and_modify(|value: &mut i64| *value = (*value).max(sequence))
                    .or_insert(sequence);
            }
        }
        for ((thread, segment, connection, epoch), sequence) in watermarks {
            tx.execute("INSERT INTO companion_route_sequences(thread_id,provider_segment_id,connection_id,epoch,sequence) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(thread_id,provider_segment_id,connection_id) DO UPDATE SET sequence=CASE WHEN epoch=excluded.epoch THEN MAX(sequence,excluded.sequence) ELSE excluded.sequence END,epoch=excluded.epoch",params![thread,segment,connection,epoch,sequence])?;
        }
        tx.commit()?;
        Ok(accepted)
    }
}
