use std::{
	convert::Infallible,
	fmt,
	num::NonZeroU8,
	sync::{
		Arc,
		atomic::{AtomicBool, Ordering}
	},
	time::Duration
};

use tokio::{task::yield_now, time::advance};

use crate::{
	CacheEntryStatus, MutateOptions, Options, Persisted, SWR,
	cache::CacheEntry,
	fetcher::mock::{Fetcher, Key},
	hook::MockHook,
	runtime::Tokio
};

#[must_use]
fn inspect_entry<E, R, F: FnOnce(&CacheEntry<Fetcher<E>, Tokio>) -> R>(swr: &SWR<Fetcher<E>, Tokio>, key: Key, f: F) -> Option<R>
where
	E: std::error::Error + Default + Sync + Send + 'static
{
	let cache = swr.cache();
	let slot = cache.get(&key)?;
	let states = cache.states();
	let entry = states.get(slot)?;
	Some(f(entry))
}

#[tokio::test(start_paused = true)]
async fn liveness() {
	let hook = MockHook::default();
	let swr = SWR::new_in(Fetcher::new(), Tokio, hook.clone());

	hook.within(|| {
		let _ = swr.get_with::<usize, _>(&Key::Basic, Options::immutable());
	});

	inspect_entry(&swr, Key::Basic, |entry| {
		let status = entry.status();
		assert!(status.get(CacheEntryStatus::ALIVE, Ordering::Acquire));
	})
	.unwrap();

	hook.end_frame();

	inspect_entry(&swr, Key::Basic, |entry| {
		let status = entry.status();
		assert!(!status.get(CacheEntryStatus::ALIVE, Ordering::Acquire));
	})
	.unwrap();
}

#[tokio::test(start_paused = true)]
async fn garbage_collection() {
	let hook = MockHook::default();
	let swr = SWR::new_in(Fetcher::new(), Tokio, hook.clone());

	hook.within(|| {
		let _ = swr.get_with::<usize, _>(&Key::Basic, Options {
			garbage_collect_timeout: Some(Duration::from_secs(5)),
			..Options::immutable()
		});
	});

	inspect_entry(&swr, Key::Basic, |entry| {
		let status = entry.status();
		assert!(status.get(CacheEntryStatus::ALIVE, Ordering::Acquire));
	})
	.unwrap();

	hook.end_frame();

	inspect_entry(&swr, Key::Basic, |entry| {
		let status = entry.status();
		assert!(!status.get(CacheEntryStatus::ALIVE, Ordering::Acquire));
	})
	.unwrap();

	advance(Duration::from_secs(5)).await;

	hook.end_frame();

	assert!(inspect_entry(&swr, Key::Basic, |_| {}).is_none());
}

#[tokio::test(start_paused = true)]
async fn request_redraw() {
	let hook = MockHook::default();
	let swr = SWR::new_in(Fetcher::new(), Tokio, hook.clone());

	hook.within(|| {
		let res = swr.get_with::<usize, _>(&Key::Basic, Options::immutable());
		assert!(res.loading);
	});
	assert!(!hook.take_wants_redraw());

	yield_now().await;

	assert!(hook.take_wants_redraw());
	inspect_entry(&swr, Key::Basic, |entry| {
		let status = entry.status();
		assert!(!status.get(CacheEntryStatus::LOADING, Ordering::Acquire));
		assert!(status.get(CacheEntryStatus::HAS_DATA, Ordering::Acquire));
	})
	.unwrap();
}

#[tokio::test(start_paused = true)]
async fn refresh() {
	let hook = MockHook::default();
	let fetcher = Fetcher::new();
	let swr = SWR::new_in(fetcher.clone(), Tokio, hook.clone());

	hook.within(|| {
		swr.get_with::<usize, _>(&Key::Basic, Options {
			refresh_interval: Some(Duration::from_secs(5)),
			..Options::immutable()
		});
	});

	hook.set_focused(true);
	for _ in 0..3 {
		yield_now().await;
		advance(Duration::from_secs(5)).await;
	}

	assert_eq!(fetcher.fetch_count(), 3);
}

#[tokio::test(start_paused = true)]
async fn retry() {
	let hook = MockHook::default();
	let fetcher = Fetcher::new();
	let swr = SWR::new_in(fetcher.clone(), Tokio, hook.clone());

	let key = Key::ErrorNTimes(3);
	hook.within(|| {
		swr.get_with::<usize, _>(&key, Options {
			error_retry_interval: Some(Duration::from_secs(3)),
			error_retry_count: Some(NonZeroU8::new(3).unwrap()),
			..Options::immutable()
		});
	});

	yield_now().await;

	hook.set_focused(true);
	for _ in 0..3 {
		inspect_entry(&swr, key, |entry| {
			let status = entry.status();
			assert!(status.get(CacheEntryStatus::HAS_ERROR, Ordering::Acquire));
		})
		.unwrap();

		advance(Duration::from_secs(3)).await;
		yield_now().await;
	}

	inspect_entry(&swr, key, |entry| {
		let status = entry.status();
		assert!(!status.get(CacheEntryStatus::HAS_ERROR, Ordering::Acquire));
	})
	.unwrap();

	assert_eq!(fetcher.fetch_count(), 4);
}

#[tokio::test(start_paused = true)]
async fn drop_values() {
	static DATA_DROP_FLAG: AtomicBool = AtomicBool::new(false);
	static ERR_DROP_FLAG: AtomicBool = AtomicBool::new(false);

	#[derive(serde::Deserialize)]
	struct DataWithDrop;

	impl Drop for DataWithDrop {
		fn drop(&mut self) {
			DATA_DROP_FLAG.store(true, Ordering::Relaxed);
		}
	}

	#[derive(Default, Debug)]
	pub struct ErrorWithDrop;

	impl fmt::Display for ErrorWithDrop {
		fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
			f.write_str("error")
		}
	}

	impl std::error::Error for ErrorWithDrop {}

	impl Drop for ErrorWithDrop {
		fn drop(&mut self) {
			ERR_DROP_FLAG.store(true, Ordering::Relaxed);
		}
	}

	let hook = MockHook::default();
	let fetcher = Fetcher::<ErrorWithDrop>::default();
	let swr = SWR::new_in(fetcher.clone(), Tokio, hook.clone());

	let persisted = swr.persisted::<DataWithDrop, _>(&Key::AlwaysError, Options {
		fetch_on_first_use: false,
		..Default::default()
	});

	async fn set(p: &Persisted<DataWithDrop, Fetcher<ErrorWithDrop>, Tokio>, ok: bool) {
		if ok {
			let _ = p
				.mutate_with(MutateOptions::default(), move |_, _| async move { Ok::<_, Infallible>(Arc::new(DataWithDrop)) })
				.await
				.unwrap();
		} else {
			p.revalidate();
			let _ = p.get();

			yield_now().await;
		}
	}

	set(&persisted, true).await;
	assert!(!DATA_DROP_FLAG.load(Ordering::Relaxed));
	assert!(!ERR_DROP_FLAG.swap(false, Ordering::Relaxed));

	set(&persisted, false).await;
	assert!(!DATA_DROP_FLAG.load(Ordering::Relaxed));
	assert!(!ERR_DROP_FLAG.swap(false, Ordering::Relaxed));

	set(&persisted, true).await;
	assert!(DATA_DROP_FLAG.load(Ordering::Relaxed));
	assert!(ERR_DROP_FLAG.swap(false, Ordering::Relaxed));

	set(&persisted, true).await;
	assert!(DATA_DROP_FLAG.load(Ordering::Relaxed));
	assert!(!ERR_DROP_FLAG.swap(false, Ordering::Relaxed));
}
