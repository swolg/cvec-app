use super::*;
use crate::core::companion::chat_text_fits;

fn validate_scope(scope: &str) -> Result<(), Error> {
    if scope.is_empty() || scope.len() > 1024 {
        return Err(Error::App("invalid local draft scope".into()));
    }
    Ok(())
}

impl Database {
    pub fn companion_read_draft(
        &self,
        repository_id: i64,
        scope: &str,
    ) -> Result<Option<String>, Error> {
        validate_scope(scope)?;
        let connection = self.connect()?;
        Ok(connection
            .query_row(
                "SELECT text FROM companion_drafts WHERE repository_id = ?1 AND scope = ?2",
                params![repository_id, scope],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn companion_write_draft(
        &self,
        repository_id: i64,
        scope: &str,
        text: &str,
    ) -> Result<(), Error> {
        validate_scope(scope)?;
        if !chat_text_fits(text) {
            return Err(Error::App("local draft is too long".into()));
        }
        self.connect()?.execute(
            "INSERT INTO companion_drafts (repository_id, scope, text) VALUES (?1, ?2, ?3)
             ON CONFLICT(repository_id, scope) DO UPDATE SET text = excluded.text",
            params![repository_id, scope, text],
        )?;
        Ok(())
    }
}
