use super::*;
use crate::domain::{PendingSpecialSend, SpecialSend};

impl Storage {
    pub fn enqueue_special(&self, send: &SpecialSend) -> Result<i64> {
        send.validate().map_err(|_| Error::InvalidQuery)?;
        let payload = encode(send)?;
        loop {
            let random_id = fresh_special_id(&self.connection)?;
            let inserted = self.connection.execute(
                "INSERT OR IGNORE INTO special_outbox(random_id,payload) SELECT ?1,?2
                 WHERE NOT EXISTS(SELECT 1 FROM outgoing WHERE random_id=?1)
                 AND NOT EXISTS(SELECT 1 FROM media_outbox_items WHERE random_id=?1)",
                params![random_id, payload],
            )?;
            if inserted == 1 {
                return Ok(random_id);
            }
        }
    }

    pub fn pending_special(&self) -> Result<Vec<PendingSpecialSend>> {
        let mut statement = self.connection.prepare(
            "SELECT random_id,payload,uncertain FROM special_outbox
             WHERE acknowledged=0 ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (random_id, payload, uncertain) = row?;
            Ok(PendingSpecialSend {
                random_id,
                send: decode(&payload)?,
                uncertain,
            })
        })
        .collect()
    }

    pub fn pending_special_send(&self, random_id: i64) -> Result<Option<PendingSpecialSend>> {
        self.connection
            .query_row(
                "SELECT payload,uncertain FROM special_outbox
                 WHERE random_id=?1 AND acknowledged=0",
                [random_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?
            .map(|(payload, uncertain)| {
                Ok(PendingSpecialSend {
                    random_id,
                    send: decode(&payload)?,
                    uncertain,
                })
            })
            .transpose()
    }

    pub fn mark_special_attempted(&self, random_id: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE special_outbox SET uncertain=1 WHERE random_id=?1",
            [random_id],
        )?;
        Ok(())
    }
}

fn fresh_special_id(connection: &Connection) -> Result<i64> {
    use rand::{TryRng, rngs::SysRng};
    loop {
        let random_id = SysRng.try_next_u64().map_err(|_| Error::InvalidQuery)? as i64;
        if random_id == 0 {
            continue;
        }
        let exists = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM special_outbox WHERE random_id=?1)
             OR EXISTS(SELECT 1 FROM outgoing WHERE random_id=?1)
             OR EXISTS(SELECT 1 FROM media_outbox_items WHERE random_id=?1)",
            [random_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Ok(random_id);
        }
    }
}
