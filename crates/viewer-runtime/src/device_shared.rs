
use eframe::wgpu;
use std::any::Any;
use std::sync::{Arc, Mutex, Weak};

type Entry = (
    wgpu::Device,
    Box<dyn Any + Send>,
    Weak<dyn Any + Send + Sync>,
);

static SHARED: Mutex<Vec<Entry>> = Mutex::new(Vec::new());

pub fn shared<K, T>(device: &wgpu::Device, key: K, make: impl FnOnce() -> T) -> Arc<T>
where
    K: PartialEq + Send + 'static,
    T: Send + Sync + 'static,
{
    let mut retired = shader_pipeline::gpu::Retired(Vec::new());
    let mut entries = SHARED.lock().unwrap_or_else(|error| error.into_inner());
    retired.0.extend(entries.extract_if(.., |(_, _, held)| held.strong_count() == 0));
    if let Some(found) = entries.iter().find_map(|(owner, cached, held)| {
        (owner == device && cached.downcast_ref::<K>() == Some(&key))
            .then(|| held.upgrade())
            .flatten()
            .and_then(|held| held.downcast::<T>().ok())
    }) {
        return found;
    }
    let made = Arc::new(make());
    let held: Arc<dyn Any + Send + Sync> = made.clone();
    entries.push((device.clone(), Box::new(key), Arc::downgrade(&held)));
    made
}
