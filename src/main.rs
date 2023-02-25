use std::{
    future::Future,
    mem::ManuallyDrop,
    pin::Pin,
    sync::{Arc, Condvar, Mutex},
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
    thread::JoinHandle,
    time::Duration,
};

fn main() {
    block_on(Wait3s::default());
}

#[derive(Default)]
struct Wait3s {
    token: Option<JoinHandle<()>>,
}

impl Future for Wait3s {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(token) = self.token.take() {
            if token.is_finished() {
                token.join().unwrap();
                Poll::Ready(())
            } else {
                self.token = Some(token);
                Poll::Pending
            }
        } else {
            let waker = cx.waker().clone();

            self.token = Some(std::thread::spawn(|| {
                std::thread::sleep(Duration::from_secs(3));
                waker.wake();
            }));

            Poll::Pending
        }
    }
}

fn block_on<Fut>(fut: Fut)
where
    Fut: std::future::Future<Output = ()>,
{
    let mut pinned_fut = Box::pin(fut);

    let vtable = Box::new(RawWakerVTable::new(
        vt_clone,
        vt_wake,
        vt_wake_by_ref,
        vt_drop,
    ));
    let vtable = Box::leak(vtable);

    let data = Arc::new(Data {
        vtable,
        woken: Mutex::new(false),
        cond: Condvar::new(),
    });
    let ptr = Arc::into_raw(data.clone());

    let raw_waker = RawWaker::new(ptr.cast(), vtable);
    let waker = unsafe { Waker::from_raw(raw_waker) };
    let mut cx = Context::from_waker(&waker);

    while let Poll::Pending = pinned_fut.as_mut().poll(&mut cx) {
        while !data.reset_woken() {}
    }
}

unsafe fn vt_clone(ptr: *const ()) -> RawWaker {
    let data = unsafe { Data::from_raw_manually_drop(ptr) };

    let data_clone = (*data).clone();
    let waker_clone = RawWaker::new(Arc::into_raw(data_clone).cast(), data.vtable);

    waker_clone
}

unsafe fn vt_wake(ptr: *const ()) {
    let data = unsafe { Data::from_raw(ptr) };
    data.wake();
}

unsafe fn vt_wake_by_ref(ptr: *const ()) {
    let data = unsafe { Data::from_raw_manually_drop(ptr) };
    data.wake();
}

unsafe fn vt_drop(ptr: *const ()) {
    // will drop
    unsafe { Data::from_raw(ptr) };
}

struct Data {
    vtable: &'static RawWakerVTable,
    cond: Condvar,
    woken: Mutex<bool>,
}

impl Data {
    unsafe fn from_raw(ptr: *const ()) -> Arc<Self> {
        unsafe { Arc::from_raw(ptr as *const Self) }
    }

    unsafe fn from_raw_manually_drop(ptr: *const ()) -> ManuallyDrop<Arc<Self>> {
        ManuallyDrop::new(Self::from_raw(ptr))
    }

    fn wake(&self) {
        let mut woken_lock = self.woken.lock().unwrap();
        *woken_lock = true;

        self.cond.notify_one();
    }

    fn reset_woken(&self) -> bool {
        let mut woken_lock = self.cond.wait(self.woken.lock().unwrap()).unwrap();
        if *woken_lock {
            *woken_lock = false;
            true
        } else {
            false
        }
    }
}
