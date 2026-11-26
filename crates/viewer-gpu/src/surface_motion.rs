use eframe::wgpu;
use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Default)]
pub struct Coverage {
    resources: Option<Queries>,
    pending: Option<Receiver<Result<(), wgpu::BufferAsyncError>>>,
    used: Cell<u32>,
    sampling: bool,
    known: Option<bool>,
    obsolete: bool,
}

struct Queries {
    set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
}

impl Queries {
    fn new(device: &wgpu::Device, count: u32) -> Self {
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: u64::from(count) * u64::from(wgpu::QUERY_SIZE),
                usage,
                mapped_at_creation: false,
            })
        };
        Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("visible animated map surfaces"),
                ty: wgpu::QueryType::Occlusion,
                count,
            }),
            resolve: buffer(
                "animated surface coverage",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            read: buffer(
                "animated surface visibility",
                wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            ),
        }
    }
}

impl Coverage {
    pub fn begin(&mut self, device: &wgpu::Device, changed: bool, views: usize, tracking: bool) {
        self.sampling = false;
        if changed || !tracking {
            self.known = (!tracking).then_some(false);
            self.obsolete = self.pending.is_some();
        }
        if let Some(pending) = &self.pending {
            let _ = device.poll(wgpu::PollType::Poll);
            let result = match pending.try_recv() {
                Ok(Ok(())) => {
                    let queries = self.resources.as_ref().unwrap();
                    let visible = queries
                        .read
                        .slice(..u64::from(self.used.get()) * u64::from(wgpu::QUERY_SIZE))
                        .get_mapped_range()
                        .map_or(true, |bytes| bytes.iter().any(|&byte| byte != 0));
                    queries.read.unmap();
                    visible
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => true,
                Err(TryRecvError::Empty) => return,
            };
            self.pending = None;
            if !self.obsolete {
                self.known = Some(result);
            }
        }
        if !tracking || self.known.is_some() {
            return;
        }
        let count = u32::try_from(views)
            .unwrap_or(u32::MAX)
            .saturating_mul(5)
            .max(self.used.get());
        if count == 0 {
            self.known = Some(false);
            return;
        }
        if count > wgpu::QUERY_SET_MAX_QUERIES {
            self.known = Some(true);
            return;
        }
        if self
            .resources
            .as_ref()
            .is_none_or(|q| q.set.count() < count)
        {
            self.resources = Some(Queries::new(device, count));
        }
        self.obsolete = false;
        self.used.set(0);
        self.sampling = true;
    }

    pub fn query(&self) -> Option<(&wgpu::QuerySet, u32)> {
        if !self.sampling {
            return None;
        }
        let index = self.used.get();
        self.used.set(index.saturating_add(1));
        let set = &self.resources.as_ref().unwrap().set;
        (index < set.count()).then_some((set, index))
    }

    pub fn finish(&self, encoder: &mut wgpu::CommandEncoder) {
        let used = self.used.get();
        if self.sampling && used > 0 {
            let queries = self.resources.as_ref().unwrap();
            if used > queries.set.count() {
                return;
            }
            encoder.resolve_query_set(&queries.set, 0..used, &queries.resolve, 0);
            encoder.copy_buffer_to_buffer(
                &queries.resolve,
                0,
                &queries.read,
                0,
                u64::from(used) * u64::from(wgpu::QUERY_SIZE),
            );
        }
    }

    pub fn submitted(&mut self) {
        if !self.sampling {
            return;
        }
        self.sampling = false;
        let used = self.used.get();
        if used == 0 {
            self.known = Some(false);
            return;
        }
        if used > self.resources.as_ref().unwrap().set.count() {
            return;
        }
        let (send, recv) = mpsc::channel();
        self.resources
            .as_ref()
            .unwrap()
            .read
            .slice(..u64::from(used) * u64::from(wgpu::QUERY_SIZE))
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.pending = Some(recv);
    }

    pub fn live(&self) -> bool {
        self.known
            .unwrap_or(self.pending.is_some() || self.used.get() > 0)
    }
}
