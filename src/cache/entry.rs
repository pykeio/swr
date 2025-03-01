use std::{
	any::{Any, TypeId},
	sync::{
		Arc,
		atomic::{AtomicBool, AtomicU32, Ordering}
	}
};

use parking_lot::RwLock;

use crate::{
	error::MismatchedTypeError,
	fetcher::Fetcher,
	options::Options,
	revalidate::RevalidateIntent,
	runtime::Runtime,
	util::{AtomicInstant, Instant, TaskSlot}
};

pub struct CacheEntry<F: Fetcher, R: Runtime> {
	pub key: F::Key,
	data: Option<CacheEntryData>,
	pub error: Option<Arc<F::Error>>,
	pub loading: bool,
	pub validating: bool,

	pub revalidate_intent: RevalidateIntent,
	pub used_this_pass: AtomicBool,
	pub alive: AtomicBool,
	pub last_draw_time: AtomicInstant,
	pub last_request_time: Option<Instant>,

	pub fetch_task: TaskSlot<R>,
	pub refresh_task: TaskSlot<R>,
	pub(crate) retry_count: AtomicU32,
	pub retry_task: TaskSlot<R>,

	pub(crate) persisted_instances: AtomicU32,
	pub options: RwLock<Options<(), F>>
}

impl<F: Fetcher, R: Runtime> CacheEntry<F, R> {
	pub fn new(runtime: R, key: F::Key) -> Self {
		Self {
			key,
			data: None,
			error: None,
			loading: false,
			validating: false,

			revalidate_intent: RevalidateIntent::default(),
			used_this_pass: AtomicBool::new(false),
			alive: AtomicBool::new(false),
			last_draw_time: AtomicInstant::now(),
			last_request_time: None,

			fetch_task: TaskSlot::new(runtime.clone()),
			refresh_task: TaskSlot::new(runtime.clone()),
			retry_count: AtomicU32::new(0),
			retry_task: TaskSlot::new(runtime),

			persisted_instances: AtomicU32::new(0),
			options: RwLock::new(Options::default())
		}
	}

	pub fn data<T: Send + Sync + 'static>(&self) -> Option<Result<Arc<F::Response<T>>, MismatchedTypeError>> {
		self.data.as_ref().map(|data| match Arc::downcast(data.value.clone()) {
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
	}

	#[inline]
	pub fn has_data(&self) -> bool {
		self.data.is_some()
	}

	pub fn insert<T: Send + Sync + 'static>(&mut self, data: Arc<F::Response<T>>) {
		self.insert_untyped(
			data as _,
			#[cfg(debug_assertions)]
			std::any::type_name::<T>()
		);
	}

	pub fn insert_untyped(&mut self, data: Arc<dyn Any + Send + Sync>, #[cfg(debug_assertions)] type_name: &'static str) {
		self.loading = false;
		self.validating = false;

		self.data = Some(CacheEntryData {
			value: data,
			#[cfg(debug_assertions)]
			type_name
		});

		self.error.take();
		self.retry_count.store(0, Ordering::Release);

		self.last_request_time.replace(Instant::now());
	}

	pub fn insert_error(&mut self, error: Arc<F::Error>) {
		self.loading = false;
		self.validating = false;

		self.error = Some(error);
		self.last_request_time.replace(Instant::now());
	}

	pub fn swap<T: Send + Sync + 'static>(&mut self, data: Arc<F::Response<T>>) -> Option<CacheEntryData> {
		let old_data = self.data.take();
		self.insert_untyped(
			data,
			#[cfg(debug_assertions)]
			std::any::type_name::<T>()
		);
		old_data
	}

	pub fn mark_used(&self) {
		self.last_draw_time.store_now(Ordering::Release);
		self.used_this_pass.store(true, Ordering::Release);
	}
}

pub struct CacheEntryData {
	pub value: Arc<dyn Any + Send + Sync>,
	#[cfg(debug_assertions)]
	pub type_name: &'static str
}
