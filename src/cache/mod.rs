use std::{borrow::Borrow, collections::HashMap, hash::Hash};

use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use slotmap::SlotMap;

mod entry;
mod slot;
pub(crate) use self::{entry::CacheEntry, slot::CacheSlot};
use crate::{fetcher::Fetcher, runtime::Runtime};

pub struct Cache<F: Fetcher, R: Runtime> {
	runtime: R,
	pub(super) key_to_slot: RwLock<HashMap<F::Key, CacheSlot<F>>>,
	states: RwLock<SlotMap<CacheSlot<F>, CacheEntry<F, R>>>
}

impl<F: Fetcher, R: Runtime> Cache<F, R> {
	pub fn new(runtime: R) -> Self {
		Self {
			runtime,
			key_to_slot: RwLock::new(HashMap::new()),
			states: RwLock::new(SlotMap::with_key())
		}
	}

	pub fn get<K>(&self, key: &K) -> Option<CacheSlot<F>>
	where
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K>
	{
		let key_to_slot = self.key_to_slot.upgradable_read();
		key_to_slot.get(key).copied()
	}

	pub fn get_or_create<K>(&self, key: &K) -> CacheSlot<F>
	where
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K> + for<'k> From<&'k K>
	{
		let key_to_slot = self.key_to_slot.upgradable_read();
		match key_to_slot.get(key) {
			Some(slot) => *slot,
			None => {
				let mut key_to_slot = RwLockUpgradableReadGuard::upgrade(key_to_slot);

				let mut results = self.states.write();
				let slot = results.insert(CacheEntry::new(self.runtime.clone(), F::Key::from(key)));

				key_to_slot.insert(F::Key::from(key), slot);
				slot
			}
		}
	}

	pub fn states(&self) -> StateAccessor<'_, F, R> {
		StateAccessor { inner: self.states.upgradable_read() }
	}
}

pub struct StateAccessor<'c, F: Fetcher, R: Runtime> {
	inner: RwLockUpgradableReadGuard<'c, SlotMap<CacheSlot<F>, CacheEntry<F, R>>>
}

impl<F: Fetcher, R: Runtime> StateAccessor<'_, F, R> {
	pub fn get(&self, slot: CacheSlot<F>) -> Option<&CacheEntry<F, R>> {
		self.inner.get(slot)
	}

	pub fn mutate<M, T>(&mut self, slot: CacheSlot<F>, mutator: M) -> Option<T>
	where
		M: FnOnce(&mut CacheEntry<F, R>) -> T
	{
		self.inner.with_upgraded(|states| states.get_mut(slot).map(mutator))
	}

	pub fn retain<I: FnMut(CacheSlot<F>, &mut CacheEntry<F, R>) -> bool>(&mut self, cb: I) {
		self.inner.with_upgraded(|states| states.retain(cb));
	}
}
