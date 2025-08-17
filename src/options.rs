use std::{
	num::{NonZeroU8, NonZeroU32},
	sync::Arc,
	time::Duration
};

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
pub struct Options<T: Send + Sync + 'static> {
	/// Initial data to return until the cache is populated by a fetch.
	pub fallback: Option<Arc<T>>,
	/// Whether or not to perform a fetch when this key is used for the first time.
	pub fetch_on_first_use: bool,
	/// The length of time it takes after this key falls out of use for its entry to be garbage collected.
	pub garbage_collect_timeout: Option<Duration>,
	/// Whether or not to perform revalidation on this key when the application becomes focused.
	///
	/// Focus-triggered revalidations will be throttled according to [`Options::focus_throttle_interval`].
	pub revalidate_on_focus: bool,
	/// The amount of time to throttle revalidations between focus events.
	pub focus_throttle_interval: Option<Duration>,
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
	pub error_retry_count: Option<NonZeroU8>,
	/// An optional amount of time to throttle between requests.
	pub throttle: Option<Duration>
}

impl<T: Send + Sync + 'static> Default for Options<T> {
	fn default() -> Self {
		Self {
			fallback: None,
			fetch_on_first_use: true,
			garbage_collect_timeout: Some(Duration::from_secs(600)),
			revalidate_on_focus: true,
			focus_throttle_interval: Some(Duration::from_secs(5)),
			refresh_interval: None,
			refresh_when_unfocused: false,
			error_retry_interval: Some(Duration::from_secs(5)),
			error_retry_count: Some(NonZeroU8::new(5).unwrap()),
			throttle: Some(Duration::from_secs(2))
		}
	}
}

impl<T: Send + Sync + 'static> Options<T> {
	/// Default [`Options`] for resources which are expected to never update throughout the duration of the
	/// application.
	///
	/// This disables automatic [revalidation on focus][Options::revalidate_on_focus] and [garbage
	/// collection][Options::garbage_collect_timeout].
	#[must_use]
	pub fn immutable() -> Self {
		Self {
			revalidate_on_focus: false,
			garbage_collect_timeout: None,
			..Options::default()
		}
	}
}

#[derive(Debug, Default)]
pub(crate) struct RevalidateFlags(u8);

impl RevalidateFlags {
	pub const ON_FIRST_USE: u8 = 1 << 0;
	pub const ON_FOCUS: u8 = 1 << 1;
	pub const WHEN_UNFOCUSED: u8 = 1 << 2;

	pub fn get(&self, bits: u8) -> bool {
		(self.0 & bits) != 0
	}
	pub fn set(&mut self, bits: u8) {
		self.0 |= bits;
	}
}

pub(crate) struct StoredOptions {
	pub revalidate_flags: RevalidateFlags,
	pub error_retry_count: Option<NonZeroU8>,
	// `Duration` is 16 bytes and we definitely don't require sub-millisecond precision
	garbage_collect_timeout_ms: Option<NonZeroU32>,
	focus_throttle_interval_ms: Option<NonZeroU32>,
	refresh_interval_ms: Option<NonZeroU32>,
	error_retry_interval_ms: Option<NonZeroU32>,
	throttle_ms: Option<NonZeroU32>
}

impl Default for StoredOptions {
	fn default() -> Self {
		let mut options = StoredOptions {
			revalidate_flags: RevalidateFlags(0),
			error_retry_count: None,
			garbage_collect_timeout_ms: None,
			focus_throttle_interval_ms: None,
			refresh_interval_ms: None,
			error_retry_interval_ms: None,
			throttle_ms: None
		};
		// Inherit our options from the default values for `Options`
		options.update_from_inner(&Options::default());
		options
	}
}

impl StoredOptions {
	pub(crate) fn garbage_collect_timeout(&self) -> Option<Duration> {
		self.garbage_collect_timeout_ms.map(|d| Duration::from_millis(d.get() as _))
	}
	pub(crate) fn focus_throttle_interval(&self) -> Option<Duration> {
		self.focus_throttle_interval_ms.map(|d| Duration::from_millis(d.get() as _))
	}
	pub(crate) fn refresh_interval(&self) -> Option<Duration> {
		self.refresh_interval_ms.map(|d| Duration::from_millis(d.get() as _))
	}
	pub(crate) fn error_retry_interval(&self) -> Option<Duration> {
		self.error_retry_interval_ms.map(|d| Duration::from_millis(d.get() as _))
	}
	pub(crate) fn throttle(&self) -> Option<Duration> {
		self.throttle_ms.map(|d| Duration::from_millis(d.get() as _))
	}

	#[inline(always)]
	pub(crate) fn update_from<T: Send + Sync + 'static>(&mut self, options: &Options<T>) {
		// Save a bit on codegen by not specializing `update_from` for every variant of `T`.
		// transmuting from Options<T> to Options<()> is safe because the fallback field which uses T is an Arc (always usize
		// regardless of T), and we don't touch it
		self.update_from_inner(unsafe { std::mem::transmute::<&Options<T>, &Options<()>>(options) });
	}

	fn update_from_inner(&mut self, options: &Options<()>) {
		if options.fetch_on_first_use {
			self.revalidate_flags.set(RevalidateFlags::ON_FIRST_USE);
		}
		self.garbage_collect_timeout_ms = merge_min(self.garbage_collect_timeout_ms, duration_as_optional_millis(&options.garbage_collect_timeout));
		if options.fetch_on_first_use {
			self.revalidate_flags.set(RevalidateFlags::ON_FOCUS);
		}
		self.focus_throttle_interval_ms = merge_min(self.focus_throttle_interval_ms, duration_as_optional_millis(&options.focus_throttle_interval));
		self.refresh_interval_ms = merge_min(self.refresh_interval_ms, duration_as_optional_millis(&options.refresh_interval));
		if options.refresh_when_unfocused {
			self.revalidate_flags.set(RevalidateFlags::WHEN_UNFOCUSED);
		}
		self.error_retry_interval_ms = merge_min(self.error_retry_interval_ms, duration_as_optional_millis(&options.error_retry_interval));
		self.error_retry_count = merge_min(self.error_retry_count, options.error_retry_count);
		self.throttle_ms = merge_min(self.throttle_ms, duration_as_optional_millis(&options.throttle));
	}
}

fn duration_as_optional_millis(a: &Option<Duration>) -> Option<NonZeroU32> {
	a.and_then(|d| NonZeroU32::new(d.as_millis() as u32))
}

fn merge_min<T: Ord>(a: Option<T>, b: Option<T>) -> Option<T> {
	match (a, b) {
		(Some(a), Some(b)) => Some(a.min(b)),
		(Some(a), None) => Some(a),
		(None, Some(b)) => Some(b),
		(None, None) => None
	}
}

/// Options used for [`SWR::mutate_with`][crate::SWR::mutate_with].
pub struct MutateOptions<T: Send + Sync + 'static, U = T> {
	/// Intermediate data to display while the action is being performed, such as the expected result of the mutation if
	/// it were to succeed.
	///
	/// If provided, the key's data will immediately be replaced with this data. Once the action finishes, its result
	/// will become the key's data.
	///
	/// See [`MutateOptions::rollback_on_error`] to configure whether this should be rolled back to its previous
	/// value if the action fails.
	pub optimistic_data: Option<Arc<T>>,
	/// Whether or not to return the key to the data it had before optimistic data was applied if the action fails.
	pub rollback_on_error: bool,
	/// Whether or not the key should be revalidated after the action is complete.
	pub revalidate: bool,
	/// A function used to map from the action's result to the actual stored key data.
	///
	/// The function accepts a reference to the action's result, and the previous (non-optimistic) data stored in the
	/// key, if there was any.
	///
	/// This can be especially useful if the action returns a partial update of the data, in which case the `populator`
	/// can merge this data into the original value.
	///
	/// ```ignore
	/// # use std::sync::Arc;
	/// #[derive(Clone, serde::Deserialize)]
	/// struct Account {
	/// 	id: String,
	/// 	balance: usize
	/// }
	///
	/// struct TransactionResult {
	/// 	balance: usize
	/// }
	///
	/// fn populate_account_from_transaction(partial: &TransactionResult, old_data: Option<&Account>) -> Arc<Account> {
	/// 	Arc::new(Account {
	/// 		balance: partial.balance,
	/// 		..old_data.unwrap().clone()
	/// 	})
	/// }
	///
	/// # let hook = swr::hook::MockHook::default();
	/// # let swr = swr::new_in(Fetcher::new(), swr::runtime::Tokio, hook);
	/// swr.get::<Account, _>("/accounts/1234").mutate_with(
	/// 	swr::MutateOptions::default().with_populator(populate_account_from_transaction),
	/// 	|_data, _fetcher| async move { Ok::<_, usize>(TransactionResult { balance: 42 }) }
	/// );
	/// ```
	pub populator: Box<dyn Fn(&U, Option<&T>) -> Arc<T> + Send>
}

impl<T: Send + Sync + 'static> Default for MutateOptions<T, Arc<T>> {
	fn default() -> Self {
		Self {
			optimistic_data: None,
			rollback_on_error: true,
			revalidate: false,
			populator: Box::new(|x, _prev| x.clone())
		}
	}
}

impl<T: Send + Sync + 'static, U> MutateOptions<T, U> {
	/// Configures a [populator][MutateOptions::populator] using the builder pattern.
	pub fn with_populator<V>(self, populator: impl Fn(&V, Option<&T>) -> Arc<T> + Send + 'static) -> MutateOptions<T, V> {
		MutateOptions {
			optimistic_data: self.optimistic_data,
			rollback_on_error: self.rollback_on_error,
			revalidate: self.revalidate,
			populator: Box::new(populator)
		}
	}
}
