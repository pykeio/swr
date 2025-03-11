use std::{
	num::NonZeroU8,
	sync::{
		Arc,
		atomic::{AtomicU8, Ordering}
	},
	time::Duration
};

use serde::de::DeserializeOwned;

#[cfg(feature = "tracing")]
use crate::util::Instant;
use crate::{
	CacheEntryStatus, SWRInner,
	cache::{CacheEntry, CacheSlot},
	fetcher::Fetcher,
	options::RevalidateFlags,
	runtime::Runtime,
	util::{AtomicBitwise, TaskStartMode, throttle}
};

#[derive(Default)]
#[repr(transparent)]
pub struct RevalidateIntent(AtomicU8);

impl RevalidateIntent {
	pub const MANUALLY_TRIGGERED: u8 = 1 << 0;
	pub const APPLICATION_FOCUSED: u8 = 1 << 1;
	pub const RETRY_ON_ERROR: u8 = 1 << 2;
	pub const FIRST_USAGE: u8 = 1 << 3;
	pub const REFRESH_INTERVAL: u8 = 1 << 4;
	pub const STALE: u8 = 1 << 5;
	pub const MUTATE: u8 = 1 << 6;

	pub fn add(&self, flag: u8) -> bool {
		self.0.bits_set(flag, Ordering::AcqRel)
	}

	pub fn take(&self) -> u8 {
		self.0.swap(0, Ordering::AcqRel)
	}

	#[cfg(feature = "tracing")]
	pub fn describe(flags: u8) -> String {
		let mut reasons = Vec::new();
		if flags & Self::MANUALLY_TRIGGERED != 0 {
			reasons.push("manual trigger");
		}
		if flags & Self::APPLICATION_FOCUSED != 0 {
			reasons.push("application focus");
		}
		if flags & Self::RETRY_ON_ERROR != 0 {
			reasons.push("previous fetch failure (error_retry_interval)");
		}
		if flags & Self::FIRST_USAGE != 0 {
			reasons.push("first usage of key");
		}
		if flags & Self::REFRESH_INTERVAL != 0 {
			reasons.push("automatic refresh (refresh_interval)");
		}
		if flags & Self::STALE != 0 {
			reasons.push("stale data");
		}
		if flags & Self::MUTATE != 0 {
			reasons.push("mutation");
		}
		reasons.join(", ")
	}
}

pub fn launch_fetch<T, F, R>(entry: &mut CacheEntry<F, R>, inner: &Arc<SWRInner<F, R>>, slot: CacheSlot, mode: TaskStartMode, intent: u8)
where
	T: DeserializeOwned + Send + Sync + 'static,
	F: Fetcher,
	R: Runtime
{
	let inner = Arc::clone(inner);
	let key = entry.key().clone();
	let did_launch = entry.fetch_task.insert(mode, async move {
		#[cfg(feature = "tracing")]
		{
			tracing::debug!(key = ?key, "fetch triggered due to: {}", RevalidateIntent::describe(intent));
		}

		#[cfg(feature = "tracing")]
		let before = Instant::now();

		let res = inner.fetcher.fetch::<T>(&key).await;
		let mut states = inner.cache.states();
		states.mutate(slot, |state| {
			match res {
				Ok(data) => {
					#[cfg(feature = "tracing")]
					{
						tracing::info!(key = ?key, "OK {}ms", before.elapsed().as_millis());
					}

					state.insert(Arc::new(data));

					let refresh_interval = { state.options.read().refresh_interval() };
					if let Some(refresh_interval) = refresh_interval {
						launch_refresh::<T, F, R>(state, &inner, slot, refresh_interval);
					}
				}
				Err(err) => {
					#[cfg(feature = "tracing")]
					{
						tracing::info!(key = ?key, "ERR {}ms: {err}", before.elapsed().as_millis());
					}

					state.insert_error(Arc::new(err));

					let retry_count = state.retry_count.fetch_add(1, Ordering::AcqRel);
					let options = state.options.read();
					if let Some(retry_interval) = options.error_retry_interval() {
						let max_count = options.error_retry_count.map_or(0, NonZeroU8::get);
						if max_count == 0 || retry_count < max_count {
							drop(options);
							launch_retry::<T, F, R>(state, &inner, slot, retry_interval);
						}
					}
				}
			}
			inner.hook.request_redraw();
		});
	});
	if did_launch {
		let status = entry.status();
		if status.get(CacheEntryStatus::HAS_DATA, Ordering::Relaxed) {
			status.set(CacheEntryStatus::VALIDATING, Ordering::Relaxed);
		} else {
			status.set(CacheEntryStatus::LOADING, Ordering::Relaxed);
		}
	}
}

pub fn launch_refresh<T, F, R>(entry: &mut CacheEntry<F, R>, inner: &Arc<SWRInner<F, R>>, slot: CacheSlot, refresh_interval: Duration)
where
	T: DeserializeOwned + Send + Sync + 'static,
	F: Fetcher,
	R: Runtime
{
	let inner = Arc::clone(inner);
	entry.refresh_task.insert(TaskStartMode::Abort, async move {
		inner.runtime.wait(refresh_interval).await;

		let mut states = inner.cache.states();
		states.mutate(slot, |state| {
			let options = state.options.read();
			if (options.revalidate_flags.get(RevalidateFlags::WHEN_UNFOCUSED) || inner.hook.focused())
				&& state.status().get(CacheEntryStatus::ALIVE, Ordering::Acquire)
				&& throttle(state.last_request_time(Ordering::Acquire), options.throttle())
			{
				drop(options);

				launch_fetch::<T, F, R>(state, &inner, slot, TaskStartMode::Soft, RevalidateIntent::REFRESH_INTERVAL);
				inner.hook.request_redraw();

				// Fetch will automatically schedule the next refresh, so our work is done.
				return;
			}

			// We did not launch a fetch, so we have to launch the next refresh task ourselves.
			if let Some(refresh_interval) = options.refresh_interval() {
				drop(options);
				launch_refresh::<T, F, R>(state, &inner, slot, refresh_interval);
			}
		});
	});
}

pub fn launch_retry<T, F, R>(entry: &mut CacheEntry<F, R>, inner: &Arc<SWRInner<F, R>>, slot: CacheSlot, retry_interval: Duration)
where
	T: DeserializeOwned + Send + Sync + 'static,
	F: Fetcher,
	R: Runtime
{
	let inner = Arc::clone(inner);
	entry.retry_task.insert(TaskStartMode::Abort, async move {
		inner.runtime.wait(retry_interval).await;

		let mut states = inner.cache.states();
		states.mutate(slot, |state| {
			let status = state.status().load(Ordering::Acquire);
			if (status & CacheEntryStatus::HAS_ERROR == 0) || (status & CacheEntryStatus::ALIVE == 0) {
				return;
			}

			let options = state.options.read();
			if throttle(state.last_request_time(Ordering::Acquire), options.throttle()) {
				drop(options);

				launch_fetch::<T, F, R>(state, &inner, slot, TaskStartMode::Soft, RevalidateIntent::RETRY_ON_ERROR);
				inner.hook.request_redraw();
			}
		});
	});
}
