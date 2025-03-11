use std::{
	any::{Any, TypeId},
	mem::MaybeUninit,
	sync::{
		Arc,
		atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering}
	},
	time::Duration
};

use parking_lot::RwLock;

use crate::{
	error::MismatchedTypeError,
	fetcher::Fetcher,
	options::StoredOptions,
	revalidate::RevalidateIntent,
	runtime::Runtime,
	util::{AtomicBitwise, Instant, TaskSlot}
};

#[repr(transparent)]
pub struct CacheEntryStatus(AtomicU8);

impl CacheEntryStatus {
	pub const HAS_DATA: u8 = 1 << 0;
	pub const HAS_ERROR: u8 = 1 << 1;

	pub const LOADING: u8 = 1 << 2;
	pub const VALIDATING: u8 = 1 << 3;

	pub const ALIVE: u8 = 1 << 4;
	pub const USED_THIS_PASS: u8 = 1 << 5;

	pub fn new() -> Self {
		CacheEntryStatus(AtomicU8::new(0))
	}

	pub fn load(&self, ordering: Ordering) -> u8 {
		self.0.load(ordering)
	}

	pub fn set(&self, bits: u8, ordering: Ordering) -> bool {
		self.0.bits_set(bits, ordering)
	}

	pub fn get(&self, bits: u8, ordering: Ordering) -> bool {
		self.0.bits_get(bits, ordering)
	}

	pub fn clear(&self, bits: u8, ordering: Ordering) -> bool {
		self.0.bits_clear(bits, ordering)
	}
}

pub struct CacheEntry<F: Fetcher, R: Runtime> {
	key: F::Key,

	pub(crate) retry_count: AtomicU8,
	status: CacheEntryStatus,
	revalidate_intent: RevalidateIntent,
	data: MaybeUninit<CacheEntryData>,
	error: MaybeUninit<Arc<F::Error>>,

	base_time: Instant,
	// offset from base time in nanos (up to ~584 years for 64 bits)
	last_draw_time_offset: AtomicU64,
	// offset from base time in nanos where u64::MAX is None, i.e. no request has been made
	last_request_time_offset: AtomicU64,

	pub fetch_task: TaskSlot<R>,
	pub refresh_task: TaskSlot<R>,
	pub retry_task: TaskSlot<R>,

	pub(crate) strong_count: AtomicU32,
	pub options: RwLock<StoredOptions>
}

impl<F: Fetcher, R: Runtime> CacheEntry<F, R> {
	pub fn new(runtime: R, key: F::Key) -> Self {
		Self {
			key,

			retry_count: AtomicU8::new(0),
			status: CacheEntryStatus::new(),
			revalidate_intent: RevalidateIntent::default(),
			data: MaybeUninit::uninit(),
			error: MaybeUninit::uninit(),

			base_time: Instant::now(),
			last_draw_time_offset: AtomicU64::new(0),
			last_request_time_offset: AtomicU64::new(u64::MAX),

			fetch_task: TaskSlot::new(runtime.clone()),
			refresh_task: TaskSlot::new(runtime.clone()),
			retry_task: TaskSlot::new(runtime),

			strong_count: AtomicU32::new(0),
			options: RwLock::new(StoredOptions::default())
		}
	}

	pub fn data<T: Send + Sync + 'static>(&self) -> Option<Result<Arc<F::Response<T>>, MismatchedTypeError>> {
		if self.status.get(CacheEntryStatus::HAS_DATA, Ordering::Acquire) {
			let data = unsafe { self.data.assume_init_ref() };
			Some(match Arc::downcast(data.value.clone()) {
				Ok(x) => Ok(x),
				Err(_) => Err(MismatchedTypeError {
					contained_type: self.data.type_id(),
					wanted_type: TypeId::of::<T>(),

					#[cfg(debug_assertions)]
					contained_type_name: data.type_name,
					#[cfg(debug_assertions)]
					wanted_type_name: std::any::type_name::<T>()
				})
			})
		} else {
			None
		}
	}

	pub fn error(&self) -> Option<&Arc<F::Error>> {
		if self.status.get(CacheEntryStatus::HAS_ERROR, Ordering::Acquire) {
			let err = unsafe { self.error.assume_init_ref() };
			Some(err)
		} else {
			None
		}
	}

	#[inline]
	pub fn revalidate_intent(&self) -> &RevalidateIntent {
		&self.revalidate_intent
	}

	#[inline]
	pub fn status(&self) -> &CacheEntryStatus {
		&self.status
	}

	#[inline]
	pub fn key(&self) -> &F::Key {
		&self.key
	}

	pub fn insert<T: Send + Sync + 'static>(&mut self, data: Arc<F::Response<T>>) -> Option<CacheEntryData> {
		self.insert_untyped(
			data as _,
			#[cfg(debug_assertions)]
			std::any::type_name::<T>()
		)
	}

	pub fn insert_untyped(&mut self, data: Arc<dyn Any + Send + Sync>, #[cfg(debug_assertions)] type_name: &'static str) -> Option<CacheEntryData> {
		self.status
			.clear(CacheEntryStatus::LOADING | CacheEntryStatus::VALIDATING, Ordering::Relaxed); // we have mut

		let old_data = if self.status.set(CacheEntryStatus::HAS_DATA, Ordering::Relaxed) {
			Some(unsafe { self.data.assume_init_read() })
		} else {
			None
		};
		self.data.write(CacheEntryData {
			value: data,
			#[cfg(debug_assertions)]
			type_name
		});

		if self.status.clear(CacheEntryStatus::HAS_ERROR, Ordering::Relaxed) {
			unsafe { self.error.assume_init_drop() };
		}

		self.retry_count.store(0, Ordering::Relaxed);
		self.last_request_time_offset
			.store(instant_as_offset(&self.base_time, Instant::now()), Ordering::Relaxed);

		old_data
	}

	pub fn insert_error(&mut self, error: Arc<F::Error>) {
		self.status
			.clear(CacheEntryStatus::LOADING | CacheEntryStatus::VALIDATING, Ordering::Relaxed); // we have mut

		if self.status.set(CacheEntryStatus::HAS_ERROR, Ordering::Relaxed) {
			unsafe { self.error.assume_init_drop() };
		}
		self.error.write(error);

		self.last_request_time_offset
			.store(instant_as_offset(&self.base_time, Instant::now()), Ordering::Relaxed);
	}

	pub fn mark_used(&self) {
		self.last_draw_time_offset
			.store(instant_as_offset(&self.base_time, Instant::now()), Ordering::Release);
		self.status.set(CacheEntryStatus::USED_THIS_PASS, Ordering::Release);
	}

	pub fn last_request_time(&self, order: Ordering) -> Option<Instant> {
		match self.last_request_time_offset.load(order) {
			u64::MAX => None,
			offs => Some(instant_from_offset(&self.base_time, offs))
		}
	}

	pub fn last_draw_time(&self, order: Ordering) -> Instant {
		instant_from_offset(&self.base_time, self.last_draw_time_offset.load(order))
	}
}

pub struct CacheEntryData {
	pub value: Arc<dyn Any + Send + Sync>,
	#[cfg(debug_assertions)]
	pub type_name: &'static str
}

fn instant_as_offset(base: &Instant, new_value: Instant) -> u64 {
	let offset = new_value - *base;
	offset.as_secs() * 1_000_000_000 + u64::from(offset.subsec_nanos())
}

fn instant_from_offset(base: &Instant, offset_nanos: u64) -> Instant {
	let secs = offset_nanos / 1_000_000_000;
	let subsec_nanos = (offset_nanos % 1_000_000_000) as u32;
	*base + Duration::new(secs, subsec_nanos)
}
