use super::*;
use crate::domain::{AppPreferences, PreferenceDocument, PreferenceNamespace};

impl Storage {
    pub fn preferences(&self) -> Result<AppPreferences> {
        let mut documents = Vec::new();
        for namespace in PreferenceNamespace::ALL {
            if let Some(document) = self.preference(namespace)? {
                documents.push((namespace, document));
            }
        }
        Ok(AppPreferences { documents })
    }

    pub fn preference(&self, namespace: PreferenceNamespace) -> Result<Option<PreferenceDocument>> {
        self.connection
            .query_row(
                "SELECT version,payload FROM app_preferences WHERE namespace=?1",
                [namespace.as_str()],
                |row| {
                    Ok(PreferenceDocument {
                        version: row.get::<_, u32>(0)?,
                        payload_json: row.get(1)?,
                    })
                },
            )
            .optional()
    }

    pub fn set_preference(
        &self,
        namespace: PreferenceNamespace,
        document: &PreferenceDocument,
    ) -> Result<()> {
        document.validate().map_err(|_| Error::InvalidQuery)?;
        self.connection.execute(
            "INSERT INTO app_preferences(namespace,version,payload) VALUES(?1,?2,?3)
             ON CONFLICT(namespace) DO UPDATE SET version=excluded.version,payload=excluded.payload",
            params![namespace.as_str(), document.version, document.payload_json],
        )?;
        Ok(())
    }

    pub fn compare_and_swap_preference(
        &self,
        namespace: PreferenceNamespace,
        expected_version: Option<u32>,
        document: &PreferenceDocument,
    ) -> Result<bool> {
        document.validate().map_err(|_| Error::InvalidQuery)?;
        let changed = match expected_version {
            Some(expected) => self.connection.execute(
                "UPDATE app_preferences SET version=?2,payload=?3
                 WHERE namespace=?1 AND version=?4",
                params![
                    namespace.as_str(),
                    document.version,
                    document.payload_json,
                    expected
                ],
            )?,
            None => self.connection.execute(
                "INSERT OR IGNORE INTO app_preferences(namespace,version,payload)
                 VALUES(?1,?2,?3)",
                params![namespace.as_str(), document.version, document.payload_json],
            )?,
        };
        Ok(changed == 1)
    }

    pub fn delete_preference(&self, namespace: PreferenceNamespace) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM app_preferences WHERE namespace=?1",
            [namespace.as_str()],
        )? == 1)
    }
}
