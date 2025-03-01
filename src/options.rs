use std::{num::NonZeroU32, sync::Arc, time::Duration};

use crate::Fetcher;

/// # Merging behavior
/// When a key is retrieved multiple times using [`Options`], the actual options used by the cache entry will be
/// *merged*. Merging wil **OR** boolean options like [`Options::revalidate_on_focus`] and choose the **minimum**
/// for other options like [`Options::refresh_interval`].
///
/// Note that [`Options::fallback`] operates independently of the cache and thus is *local to each retrieved key*; it
/// does not apply to other usages of the key that do not specify their own fallback.
///
/// Imagine that two parts of your code use the same key, but with different options:
/// - **Options A** specifies a fallback, refreshes every 5 seconds, but does not revalidate on focus.
/// - **Options B** does not specify a fallback, refreshes every 60 seconds, and does revalidate on focus.
///
/// The actual cache entry shared between them will refresh every 5 seconds, and will trigger revalidation when the
/// application is focused. The code that requested **Options A** will have the fallback, but the code that requested
/// **Options B** will not.
#[derive(Clone, Debug)]
pub struct Options<T: Send + Sync + 'static, F: Fetcher> {
	/// Initial data to return until the cache is populated by a fetch.
	pub fallback: Option<Arc<F::Response<T>>>,
	/// Whether or not to perform a fetch when this key is used for the first time.
	pub fetch_on_first_use: bool,
	/// The length of time it takes after this key falls out of use for its entry to be garbage collected.
	pub garbage_collect_timeout: Duration,
	/// Whether or not to perform revalidation on this key when the application becomes focused.
	///
	/// Focus-triggered revalidations will be throttled according to [`Options::focus_throttle_interval`].
	pub revalidate_on_focus: bool,
	/// The amount of time to throttle revalidations between focus events.
	pub focus_throttle_interval: Duration,
	/// An optional interval at which to refresh data.
	///
	/// If [`Options::refresh_when_unfocused`] is `false` (the default), interval-based refreshes will only occur
	/// while the application is focused.
	pub refresh_interval: Option<Duration>,
	/// Whether or not to still perform interval refreshes ([`Options::refresh_interval`]) while the application is
	/// unfocused.
	///
	/// Refreshes will trigger the application to rerender when new data arrives.
	///
	/// If `refresh_interval` is `None`, this option does nothing.
	pub refresh_when_unfocused: bool,
	/// An optional interval at which to retry fetches if an error occurs.
	pub error_retry_interval: Option<Duration>,
	/// The maximum amount of times to retry fetching if an error occurs.
	pub error_retry_count: Option<NonZeroU32>,
	/// An optional amount of time to throttle between requests.
	pub throttle: Option<Duration>
}

impl<T: Send + Sync + 'static, F: Fetcher> Default for Options<T, F> {
	fn default() -> Self {
		Self {
			fallback: None,
			fetch_on_first_use: true,
			garbage_collect_timeout: Duration::from_secs(600),
			revalidate_on_focus: true,
			focus_throttle_interval: Duration::from_secs(5),
			refresh_interval: None,
			refresh_when_unfocused: false,
			error_retry_interval: Some(Duration::from_secs(5)),
			error_retry_count: Some(NonZeroU32::new(5).unwrap()),
			throttle: Some(Duration::from_secs(2))
		}
	}
}

impl<T: Send + Sync + 'static, F: Fetcher> Options<T, F> {
	/// Default [`Options`] for resources which are expected to never update throughout the duration of the
	/// application.
	///
	/// This disables automatic [revalidation on focus][Options::revalidate_on_focus] and [garbage
	/// collection][Options::garbage_collect_timeout].
	#[must_use]
	pub fn immutable() -> Self {
		Self {
			revalidate_on_focus: false,
			garbage_collect_timeout: Duration::from_secs(u64::MAX),
			..Options::default()
		}
	}

	pub(crate) fn update_from<U: Send + Sync + 'static>(&mut self, other: &Options<U, F>) {
		self.fetch_on_first_use |= other.fetch_on_first_use;
		self.garbage_collect_timeout = self.garbage_collect_timeout.min(other.garbage_collect_timeout);
		self.revalidate_on_focus |= other.revalidate_on_focus;
		self.focus_throttle_interval = self.focus_throttle_interval.min(self.focus_throttle_interval);
		self.refresh_interval = merge_min(self.refresh_interval.as_ref(), other.refresh_interval.as_ref());
		self.refresh_when_unfocused |= other.refresh_when_unfocused;
		self.error_retry_interval = merge_min(self.error_retry_interval.as_ref(), other.error_retry_interval.as_ref());
		self.error_retry_count = merge_min(self.error_retry_count.as_ref(), other.error_retry_count.as_ref());
		self.throttle = merge_min(self.throttle.as_ref(), other.throttle.as_ref());
	}
}

fn merge_min<T: Copy + Ord>(a: Option<&T>, b: Option<&T>) -> Option<T> {
	match (a, b) {
		(Some(a), Some(b)) => Some(*a.min(b)),
		(Some(a), None) => Some(*a),
		(None, Some(b)) => Some(*b),
		(None, None) => None
	}
}

pub struct MutateOptions<F: Fetcher, T: Send + Sync + 'static, U = T> {
	pub optimistic_data: Option<Arc<F::Response<T>>>,
	pub rollback_on_error: bool,
	pub revalidate: bool,
	pub populator: Box<dyn Fn(&U) -> Arc<F::Response<T>> + Send>
}

impl<F: Fetcher, T: Send + Sync + 'static> Default for MutateOptions<F, T, Arc<F::Response<T>>> {
	fn default() -> Self {
		Self {
			optimistic_data: None,
			rollback_on_error: true,
			revalidate: false,
			populator: Box::new(|x| x.clone())
		}
	}
}

impl<F: Fetcher, T: Send + Sync + 'static, U> MutateOptions<F, T, U> {
	pub fn with_populator<V>(self, populator: impl Fn(&V) -> Arc<F::Response<T>> + Send + 'static) -> MutateOptions<F, T, V> {
		MutateOptions {
			optimistic_data: self.optimistic_data,
			rollback_on_error: self.rollback_on_error,
			revalidate: self.revalidate,
			populator: Box::new(populator)
		}
	}
}
