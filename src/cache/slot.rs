use std::{
	fmt,
	hash::{Hash, Hasher},
	marker::PhantomData
};

use crate::fetcher::Fetcher;

#[repr(transparent)]
pub struct CacheSlot<F: Fetcher> {
	data: slotmap::KeyData,
	_p: PhantomData<fn(F) -> F>
}

impl<F: Fetcher> Default for CacheSlot<F> {
	fn default() -> Self {
		CacheSlot {
			data: slotmap::KeyData::default(),
			_p: PhantomData
		}
	}
}

impl<F: Fetcher> Clone for CacheSlot<F> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<F: Fetcher> Copy for CacheSlot<F> {}

impl<F: Fetcher> PartialEq for CacheSlot<F> {
	fn eq(&self, other: &Self) -> bool {
		self.data.eq(&other.data)
	}
}
impl<F: Fetcher> Eq for CacheSlot<F> {}

impl<F: Fetcher> PartialOrd for CacheSlot<F> {
	fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
		Some(self.data.cmp(&other.data))
	}
}
impl<F: Fetcher> Ord for CacheSlot<F> {
	fn cmp(&self, other: &Self) -> std::cmp::Ordering {
		self.data.cmp(&other.data)
	}
}

impl<F: Fetcher> Hash for CacheSlot<F> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.data.hash(state);
	}
}

impl<F: Fetcher> fmt::Debug for CacheSlot<F> {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		self.data.fmt(f)
	}
}

impl<F: Fetcher> From<slotmap::KeyData> for CacheSlot<F> {
	fn from(key_data: slotmap::KeyData) -> Self {
		CacheSlot { data: key_data, _p: PhantomData }
	}
}

unsafe impl<F: Fetcher> slotmap::Key for CacheSlot<F> {
	fn data(&self) -> slotmap::KeyData {
		self.data
	}
}
