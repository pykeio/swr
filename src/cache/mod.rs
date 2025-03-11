use std::{borrow::Borrow, collections::HashMap, hash::Hash};

use parking_lot::{RwLock, RwLockUpgradableReadGuard};
use slotmap::SlotMap;

mod entry;
pub(crate) use self::entry::{CacheEntry, CacheEntryStatus};
use crate::{fetcher::Fetcher, runtime::Runtime};

slotmap::new_key_type! {
	pub struct CacheSlot;
}

pub struct Cache<F: Fetcher, R: Runtime> {
	runtime: R,
	key_to_slot: RwLock<HashMap<F::Key, CacheSlot>>,
	states: RwLock<SlotMap<CacheSlot, CacheEntry<F, R>>>
}

impl<F: Fetcher, R: Runtime> Cache<F, R> {
	pub fn new(runtime: R) -> Self {
		Self {
			runtime,
			key_to_slot: RwLock::new(HashMap::new()),
			states: RwLock::new(SlotMap::with_key())
		}
	}

	pub fn get<K>(&self, key: &K) -> Option<CacheSlot>
	where
		K: Hash + Eq + ?Sized,
		F::Key: Borrow<K>
	{
		let key_to_slot = self.key_to_slot.upgradable_read();
		key_to_slot.get(key).copied()
	}

	pub fn get_or_create<K>(&self, key: &K) -> CacheSlot
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

	pub(crate) fn retain<I: FnMut(CacheSlot, &mut CacheEntry<F, R>) -> bool>(&self, mut cb: I) {
		let mut key_to_slot = self.key_to_slot.write();
		let mut states = self.states.write();
		states.retain(|slot, entry| {
			if !cb(slot, entry) {
				key_to_slot.remove(entry.key());
				false
			} else {
				true
			}
		})
	}

	pub fn states(&self) -> StateAccessor<'_, F, R> {
		StateAccessor { inner: self.states.upgradable_read() }
	}
}

pub struct StateAccessor<'c, F: Fetcher, R: Runtime> {
	inner: RwLockUpgradableReadGuard<'c, SlotMap<CacheSlot, CacheEntry<F, R>>>
}

impl<F: Fetcher, R: Runtime> StateAccessor<'_, F, R> {
	pub fn get(&self, slot: CacheSlot) -> Option<&CacheEntry<F, R>> {
		self.inner.get(slot)
	}

	pub fn mutate<M, T>(&mut self, slot: CacheSlot, mutator: M) -> Option<T>
	where
		M: FnOnce(&mut CacheEntry<F, R>) -> T
	{
		self.inner.with_upgraded(|states| states.get_mut(slot).map(mutator))
	}
}
