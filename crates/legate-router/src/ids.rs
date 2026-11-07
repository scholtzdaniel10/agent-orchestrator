//! Fixed indexes for [`ProviderId`] and [`JobType`]. Picking uses these instead of maps.

use legate_core::{JobType, PROVIDERS, ProviderId};

pub const PROVIDER_COUNT: usize = PROVIDERS.len();
pub const JOB_COUNT: usize = 5;
/// Longest provider list a compiled rule table stores.
pub const MAX_ORDER: usize = 8;

pub const fn provider_index(id: ProviderId) -> usize {
    match id {
        ProviderId::Claude => 0,
        ProviderId::Cursor => 1,
        ProviderId::Grok => 2,
        ProviderId::Local => 3,
    }
}

#[allow(dead_code)]
pub const fn provider_from_index(index: usize) -> Option<ProviderId> {
    match index {
        0 => Some(ProviderId::Claude),
        1 => Some(ProviderId::Cursor),
        2 => Some(ProviderId::Grok),
        3 => Some(ProviderId::Local),
        _ => None,
    }
}

/// Slot of the plan a worker spends. Grok shares Cursor's slot.
pub const fn plan_index(id: ProviderId) -> usize {
    match id {
        ProviderId::Grok => provider_index(ProviderId::Cursor),
        other => provider_index(other),
    }
}

pub const fn job_index(job: JobType) -> usize {
    match job {
        JobType::Planning => 0,
        JobType::Debugging => 1,
        JobType::Review => 2,
        JobType::Refactor => 3,
        JobType::Boilerplate => 4,
    }
}

/// Which workers are eligible. Stack-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProviderSet {
    flags: [bool; PROVIDER_COUNT],
}

impl ProviderSet {
    pub fn from_slice(ids: &[ProviderId]) -> Self {
        let mut set = Self::default();
        for id in ids {
            set.flags[provider_index(*id)] = true;
        }
        set
    }

    pub const fn contains(self, id: ProviderId) -> bool {
        self.flags[provider_index(id)]
    }
}

/// Jobs already running on each worker. Indexed by [`provider_index`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RunningCounts {
    counts: [u32; PROVIDER_COUNT],
}

impl RunningCounts {
    pub const fn get(self, id: ProviderId) -> u32 {
        self.counts[provider_index(id)]
    }

    pub fn set(&mut self, id: ProviderId, count: u32) {
        self.counts[provider_index(id)] = count;
    }

    pub fn add(&mut self, id: ProviderId, count: u32) {
        self.counts[provider_index(id)] = self.counts[provider_index(id)].saturating_add(count);
    }
}

/// Jobs already running on the plan `provider` spends, across workers that share it.
pub fn jobs_on_plan(running: &RunningCounts, provider: ProviderId) -> u32 {
    let plan = legate_core::plan_of(provider);
    let mut total = 0u32;
    for id in PROVIDERS {
        if legate_core::plan_of(id) == plan {
            total += running.get(id);
        }
    }
    total
}
