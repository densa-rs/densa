//! Thread priority: set through a handle or on the current thread, verified by reading the
//! OS's value back rather than trusting the setter's return value.

use densa_platform::ThreadPriority;

#[test]
fn default_priority_is_normal() {
    assert_eq!(ThreadPriority::default(), ThreadPriority::Normal);
}

#[cfg(windows)]
mod windows {
    use std::sync::mpsc;

    use densa_platform::{
        current_thread_priority, set_current_thread_priority, set_thread_priority, thread_priority,
        ThreadPriority,
    };

    const ALL: [ThreadPriority; 4] = [
        ThreadPriority::Normal,
        ThreadPriority::AboveNormal,
        ThreadPriority::Highest,
        ThreadPriority::TimeCritical,
    ];

    #[test]
    fn new_threads_start_at_normal_priority() {
        let seen = std::thread::spawn(current_thread_priority).join().unwrap();
        assert_eq!(seen, Some(ThreadPriority::Normal));
    }

    #[test]
    fn current_thread_priority_round_trips_every_level() {
        let seen: Vec<_> = std::thread::spawn(|| {
            ALL.map(|p| {
                set_current_thread_priority(p).unwrap();
                current_thread_priority()
            })
        })
        .join()
        .unwrap()
        .into();
        assert_eq!(seen, ALL.map(Some).to_vec());
    }

    #[test]
    fn priority_set_through_a_handle_is_seen_by_the_thread() {
        let (go, wait) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            current_thread_priority()
        });
        set_thread_priority(&worker, ThreadPriority::Highest).unwrap();
        go.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), Some(ThreadPriority::Highest));
    }

    #[test]
    fn priority_can_be_read_back_through_a_handle() {
        let (done, wait) = mpsc::channel::<()>();
        let worker = std::thread::spawn(move || wait.recv().unwrap());
        set_thread_priority(&worker, ThreadPriority::AboveNormal).unwrap();
        assert_eq!(thread_priority(&worker), Some(ThreadPriority::AboveNormal));
        done.send(()).unwrap();
        worker.join().unwrap();
    }
}
