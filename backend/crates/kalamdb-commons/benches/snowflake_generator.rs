use std::{
    hint::black_box,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use criterion::{criterion_group, criterion_main, BatchSize, BenchmarkId, Criterion, Throughput};
use kalamdb_commons::ids::SnowflakeGenerator;
use parking_lot::Mutex;

const SINGLE_BURST_SIZE: usize = 1024;

struct LegacySnowflakeGenerator {
    worker_bits: u64,
    epoch:       u64,
    state:       Mutex<LegacyGeneratorState>,
}

struct LegacyGeneratorState {
    last_timestamp: u64,
    sequence:       u16,
}

impl LegacySnowflakeGenerator {
    const DEFAULT_EPOCH: u64 = SnowflakeGenerator::DEFAULT_EPOCH;
    const MAX_SEQUENCE: u16 = 4095;
    const MAX_BACKWARD_DRIFT_MS: u64 = 50;

    fn new(worker_id: u16) -> Self {
        Self {
            worker_bits: (worker_id as u64) << 12,
            epoch:       Self::DEFAULT_EPOCH,
            state:       Mutex::new(LegacyGeneratorState {
                last_timestamp: 0,
                sequence:       0,
            }),
        }
    }

    fn next_id(&self) -> Result<i64, String> {
        let mut state = self.state.lock();

        let mut timestamp = self.reconcile_timestamp(state.last_timestamp)?;

        if timestamp == state.last_timestamp {
            state.sequence = (state.sequence + 1) & Self::MAX_SEQUENCE;
            if state.sequence == 0 {
                timestamp = self.wait_next_millis(state.last_timestamp)?;
            }
        } else {
            state.sequence = 0;
        }

        state.last_timestamp = timestamp;
        Ok(self.compose_id(timestamp, state.sequence as u64))
    }

    fn next_ids(&self, count: usize) -> Result<Vec<i64>, String> {
        if count == 0 {
            return Ok(Vec::new());
        }

        let mut state = self.state.lock();
        let mut ids = Vec::with_capacity(count);

        for _ in 0..count {
            let mut timestamp = self.reconcile_timestamp(state.last_timestamp)?;

            if timestamp == state.last_timestamp {
                state.sequence = (state.sequence + 1) & Self::MAX_SEQUENCE;
                if state.sequence == 0 {
                    timestamp = self.wait_next_millis(state.last_timestamp)?;
                }
            } else {
                state.sequence = 0;
            }

            state.last_timestamp = timestamp;
            ids.push(self.compose_id(timestamp, state.sequence as u64));
        }

        Ok(ids)
    }

    fn current_timestamp(&self) -> Result<u64, String> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .map_err(|e| format!("Failed to get current timestamp: {}", e))
    }

    fn wait_next_millis(&self, last_timestamp: u64) -> Result<u64, String> {
        let mut timestamp = self.current_timestamp()?;
        while timestamp <= last_timestamp {
            timestamp = self.current_timestamp()?;
        }
        Ok(timestamp)
    }

    fn reconcile_timestamp(&self, last_timestamp: u64) -> Result<u64, String> {
        let timestamp = self.current_timestamp()?;
        if timestamp >= last_timestamp {
            return Ok(timestamp);
        }

        let drift_ms = last_timestamp - timestamp;
        if drift_ms > Self::MAX_BACKWARD_DRIFT_MS {
            return Err(format!(
                "Clock moved backwards. Refusing to generate id for {} milliseconds",
                drift_ms
            ));
        }

        thread::sleep(Duration::from_millis(drift_ms));
        self.wait_next_millis(last_timestamp)
    }

    fn compose_id(&self, timestamp: u64, sequence: u64) -> i64 {
        (((timestamp - self.epoch) << 22) | self.worker_bits | sequence) as i64
    }
}

fn bench_single_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("snowflake_single_generation");
    group.throughput(Throughput::Elements(SINGLE_BURST_SIZE as u64));

    group.bench_function(BenchmarkId::new("optimized", "single"), |b| {
        b.iter_batched(
            || SnowflakeGenerator::new(1),
            |generator| {
                for _ in 0..SINGLE_BURST_SIZE {
                    black_box(generator.next_id().expect("optimized next_id"));
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.bench_function(BenchmarkId::new("legacy", "single"), |b| {
        b.iter_batched(
            || LegacySnowflakeGenerator::new(1),
            |generator| {
                for _ in 0..SINGLE_BURST_SIZE {
                    black_box(generator.next_id().expect("legacy next_id"));
                }
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_batch_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("snowflake_batch_generation");

    for batch_size in [32_usize, 256, 1024] {
        group.throughput(Throughput::Elements(batch_size as u64));

        group.bench_with_input(
            BenchmarkId::new("optimized", batch_size),
            &batch_size,
            |b, &size| {
                b.iter_batched(
                    || SnowflakeGenerator::new(1),
                    |generator| black_box(generator.next_ids(size).expect("optimized next_ids")),
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(BenchmarkId::new("legacy", batch_size), &batch_size, |b, &size| {
            b.iter_batched(
                || LegacySnowflakeGenerator::new(1),
                |generator| black_box(generator.next_ids(size).expect("legacy next_ids")),
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

fn bench_mapped_batch_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("snowflake_mapped_batch_generation");

    for batch_size in [256_usize, 1024] {
        group.throughput(Throughput::Elements(batch_size as u64));

        group.bench_with_input(
            BenchmarkId::new("optimized_mapped", batch_size),
            &batch_size,
            |b, &size| {
                b.iter_batched(
                    || SnowflakeGenerator::new(1),
                    |generator| {
                        black_box(
                            generator
                                .next_ids_mapped(size, |id| id.wrapping_mul(31))
                                .expect("optimized next_ids_mapped"),
                        )
                    },
                    BatchSize::SmallInput,
                );
            },
        );

        group.bench_with_input(
            BenchmarkId::new("legacy_then_map", batch_size),
            &batch_size,
            |b, &size| {
                b.iter_batched(
                    || LegacySnowflakeGenerator::new(1),
                    |generator| {
                        let ids = generator.next_ids(size).expect("legacy next_ids");
                        black_box(ids.into_iter().map(|id| id.wrapping_mul(31)).collect::<Vec<_>>())
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }

    group.finish();
}

fn bench_concurrent_single_generation(c: &mut Criterion) {
    let mut group = c.benchmark_group("snowflake_concurrent_single_generation");
    let thread_count = 8;
    let ids_per_thread = 1_000;
    group.throughput(Throughput::Elements((thread_count * ids_per_thread) as u64));

    group.bench_function(
        BenchmarkId::new("optimized", format!("{}x{}", thread_count, ids_per_thread)),
        |b| {
            b.iter(|| {
                let generator = Arc::new(SnowflakeGenerator::new(1));
                let duplicates = Arc::new(AtomicU64::new(0));
                thread::scope(|scope| {
                    let mut handles = Vec::with_capacity(thread_count);
                    for _ in 0..thread_count {
                        let generator = Arc::clone(&generator);
                        let duplicates = Arc::clone(&duplicates);
                        handles.push(scope.spawn(move || {
                            let mut prev = None;
                            for _ in 0..ids_per_thread {
                                let next =
                                    generator.next_id().expect("optimized concurrent next_id");
                                if let Some(prev) = prev {
                                    if next <= prev {
                                        duplicates.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                prev = Some(next);
                            }
                        }));
                    }
                    for handle in handles {
                        handle.join().expect("optimized thread join");
                    }
                });
                assert_eq!(duplicates.load(Ordering::Relaxed), 0);
            });
        },
    );

    group.bench_function(
        BenchmarkId::new("legacy", format!("{}x{}", thread_count, ids_per_thread)),
        |b| {
            b.iter(|| {
                let generator = Arc::new(LegacySnowflakeGenerator::new(1));
                let duplicates = Arc::new(AtomicU64::new(0));
                thread::scope(|scope| {
                    let mut handles = Vec::with_capacity(thread_count);
                    for _ in 0..thread_count {
                        let generator = Arc::clone(&generator);
                        let duplicates = Arc::clone(&duplicates);
                        handles.push(scope.spawn(move || {
                            let mut prev = None;
                            for _ in 0..ids_per_thread {
                                let next = generator.next_id().expect("legacy concurrent next_id");
                                if let Some(prev) = prev {
                                    if next <= prev {
                                        duplicates.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                prev = Some(next);
                            }
                        }));
                    }
                    for handle in handles {
                        handle.join().expect("legacy thread join");
                    }
                });
                assert_eq!(duplicates.load(Ordering::Relaxed), 0);
            });
        },
    );

    group.finish();
}

criterion_group!(
    name = benches;
    config = Criterion::default().sample_size(20).warm_up_time(Duration::from_millis(500));
    targets = bench_single_generation, bench_batch_generation, bench_mapped_batch_generation, bench_concurrent_single_generation
);
criterion_main!(benches);
