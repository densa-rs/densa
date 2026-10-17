//! Exhaustive interleaving check with loom.
//!
//! Run: `RUSTFLAGS="--cfg loom" cargo test -p densa-sync --release --test loom_triple_buffer`
//! loom reports any access to a slot that is not ordered by happens-before:
//! RAW (reader sees a half-written snapshot) and WAR (writer reuses a slot the reader
//! is still reading). Each model is shaped so that the hazard it targets is reachable;
//! weakening either swap below `AcqRel` must make at least one model fail.
#![cfg(loom)]

use densa_sync::{triple_buffer, Writer};
use loom::thread;

/// Two fields written separately: a torn snapshot would show `a != b`.
#[derive(Clone, Default)]
struct Pair {
    a: u32,
    b: u32,
}

fn write_pair(writer: &mut Writer<Pair>, v: u32) {
    writer.write(|p| {
        p.a = v;
        p.b = v;
    });
}

/// RAW: every snapshot the reader takes must be fully written and never go backwards.
#[test]
fn reader_never_sees_partial_or_older_snapshot() {
    loom::model(|| {
        let (mut writer, mut reader) = triple_buffer(Pair::default());

        let producer = thread::spawn(move || {
            for v in 1..=2 {
                write_pair(&mut writer, v);
                writer.publish();
            }
        });

        let mut last = 0;
        for _ in 0..2 {
            reader.update();
            let (a, b) = reader.read(|p| (p.a, p.b));
            assert_eq!(a, b, "torn snapshot");
            assert!(a >= last, "snapshot went backwards");
            last = a;
        }

        producer.join().unwrap();
    });
}

/// WAR: the reader reads a slot, hands it back via `update`, and the writer — which keeps
/// writing after every publish — eventually receives and overwrites that same slot.
/// Without the reader's Release + writer's Acquire, loom flags the overwrite as racing
/// with the earlier read.
#[test]
fn writer_reusing_a_released_slot_is_ordered_after_reader() {
    loom::model(|| {
        let (mut writer, mut reader) = triple_buffer(Pair::default());

        let producer = thread::spawn(move || {
            for v in 1..=3 {
                write_pair(&mut writer, v);
                writer.publish();
            }
            // Unpublished write into whatever slot the last publish handed back.
            write_pair(&mut writer, 99);
        });

        for _ in 0..2 {
            let (a, b) = reader.read(|p| (p.a, p.b));
            assert_eq!(a, b, "torn snapshot");
            reader.update();
        }
        let (a, b) = reader.read(|p| (p.a, p.b));
        assert_eq!(a, b, "torn snapshot");

        producer.join().unwrap();

        reader.update();
        assert_eq!(reader.read(|p| (p.a, p.b)), (3, 3), "latest publish lost");
    });
}
