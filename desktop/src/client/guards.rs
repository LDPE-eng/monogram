use super::*;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Copy)]
pub(super) struct RequestGuard {
    generation: u64,
    scope: Option<u64>,
}

#[derive(Default)]
pub(super) struct RequestGuards {
    state: Mutex<GuardState>,
}

#[derive(Default)]
struct GuardState {
    generation: u64,
    latest: BTreeMap<u64, u64>,
}

impl RequestGuards {
    pub(super) fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
    }

    pub(super) fn register(
        &self,
        id: u64,
        expected_generation: Option<u64>,
        scope: Option<u64>,
        lifecycle: bool,
    ) -> Result<RequestGuard, ClientError> {
        let mut state = self.state.lock().map_err(|_| ClientError::Stopped)?;
        if expected_generation.is_some_and(|value| value != state.generation) {
            return Err(ClientError::Cancelled);
        }
        if lifecycle {
            state.generation = state
                .generation
                .checked_add(1)
                .ok_or(ClientError::Stopped)?;
            state.latest.clear();
        }
        if let Some(scope) = scope {
            if state
                .latest
                .get(&scope)
                .is_some_and(|previous| id <= *previous)
            {
                return Err(ClientError::InvalidRequest);
            }
            state.latest.insert(scope, id);
        }
        Ok(RequestGuard {
            generation: state.generation,
            scope,
        })
    }

    pub(super) fn current(&self, guard: RequestGuard, id: u64) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        guard.generation == state.generation
            && guard
                .scope
                .is_none_or(|scope| state.latest.get(&scope) == Some(&id))
    }
}

pub(super) fn lifecycle(command: &WorkerCommand) -> bool {
    matches!(
        command,
        WorkerCommand::Configure(_) | WorkerCommand::Logout | WorkerCommand::Destroy
    )
}

impl Worker {
    pub fn generation(&self) -> u64 {
        self.guards.generation()
    }

    pub fn submit_guarded(
        &self,
        request_id: u64,
        generation: u64,
        scope: u64,
        command: WorkerCommand,
    ) -> Result<(), ClientError> {
        if !matches!(
            &command,
            WorkerCommand::LoadChats
                | WorkerCommand::LoadMoreChats { .. }
                | WorkerCommand::LoadHistory { .. }
                | WorkerCommand::LoadCachedChats
                | WorkerCommand::LoadCachedHistory { .. }
                | WorkerCommand::PendingSends
                | WorkerCommand::ContactsSearch { .. }
                | WorkerCommand::SearchGlobal { .. }
                | WorkerCommand::SearchMessagesPage { .. }
                | WorkerCommand::GetProfile { .. }
                | WorkerCommand::GetCachedProfile { .. }
                | WorkerCommand::GetMembers { .. }
                | WorkerCommand::GetCommonChats { .. }
                | WorkerCommand::GetFolders
                | WorkerCommand::GetCachedFolders
                | WorkerCommand::GetReplies { .. }
                | WorkerCommand::GetPinnedMessages { .. }
                | WorkerCommand::GetCloudDrafts
                | WorkerCommand::GetCachedCloudDrafts
                | WorkerCommand::GetScheduledHistory { .. }
                | WorkerCommand::GetCachedScheduledHistory { .. }
        ) {
            return Err(ClientError::InvalidRequest);
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(ClientError::Stopped);
        }
        let guard = self
            .guards
            .register(request_id, Some(generation), Some(scope), false)?;
        self.commands
            .send(Request {
                id: request_id,
                command,
                guard,
            })
            .map_err(|_| ClientError::Stopped)
    }
}
