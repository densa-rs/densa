//! Real-thread stress test: detects torn reads and out-of-order snapshots.
#![cfg(not(loom))]

use densa_sync::triple_buffer;
use std::thread;

/// Large enough (4 KiB) that a torn read would very likely be observed.
const PAYLOAD_WORDS: usize = 512;
const PUBLISHES: u64 = if cfg!(miri) { 50 } else { 100_000 };

#[test]
fn reader_never_observes_torn_or_stale_snapshots() {
    let (mut writer, mut reader) = triple_buffer([0u64; PAYLOAD_WORDS]);

    let producer = thread::spawn(move || {
        for seq in 1..=PUBLISHES {
            writer.write(|buf| buf.fill(seq));
            writer.publish();
        }
    });

    let mut last_seq = 0u64;
    let mut snapshots_seen = 0u64;
    while last_seq < PUBLISHES {
        if !reader.update() {
            std::hint::spin_loop();
            continue;
        }
        let seq = reader.read(|buf| {
            let first = buf[0];
            assert!(
                buf.iter().all(|&w| w == first),
                "torn read: snapshot mixes sequence numbers"
            );
            first
        });
        assert!(
            seq > last_seq,
            "snapshot went backwards: {seq} after {last_seq}"
        );
        last_seq = seq;
        snapshots_seen += 1;
    }

    producer.join().expect("producer panicked");
    assert_eq!(last_seq, PUBLISHES, "final publish must become visible");
    assert!(snapshots_seen >= 1);
}

#[test]
fn values_cross_threads_both_directions_of_ownership() {
    // Heap-owning payload: exercises Send of non-Copy data through all three slots.
    let (mut writer, mut reader) = triple_buffer(vec![0u32; 64]);
    let producer = thread::spawn(move || {
        for i in 1..=1_000u32 {
            writer.write(|v| v.iter_mut().for_each(|x| *x = i));
            writer.publish();
        }
    });
    producer.join().expect("producer panicked");
    assert!(reader.update());
    assert!(reader.read(|v| v.iter().all(|&x| x == 1_000)));
}
