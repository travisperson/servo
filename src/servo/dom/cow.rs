/*!

Implements the copy-on-write DOM-sharing model.  This model allows for
a single writer and any number of readers, but the writer must be able
to control and manage the lifetimes of the reader(s).  For simplicity
I will describe the implementation as though there were a single
reader.

The basic idea is that every object in the COW pool has both a reader
view and a writer view.  The writer always sees the writer view, which
contains the most up-to-date values.  The reader uses the reader view,
which contains the values as of the point where the reader was forked.
When the writer joins the reader, the reader view will be synchronized
with the writer view.

Internally, the way this works is using a copy-on-write scheme.  Each
COW node maintains two pointers (`read_ptr` and `write_ptr`).
Assuming that readers are active, when a writer wants to modify a
node, it first copies the reader's data into a new pointer.  Any
writes that occur after that point (but before the reader is joined)
will operate on this same copy.  When the reader is joined, any nodes
which the writer modified will free the stale reader data and update
the reader pointer to be the same as the writer pointer.

# Using the COW APIs as a writer

You must first create a `scope` object.  The scope object manages the
memory and the COW operations.  COW'd objects of some sendable type
`T` are not referenced directly but rather through a `handle<T>`.  To
create a new COW object, you use `scope.handle(t)` where `t` is some
initial value of type `T`.  To write to an COW object, use
`scope.write()` and to read from it use `scope.read()`. Be sure not to
use the various `ReaderMethods`.

Handles can be freely sent between tasks but the COW scope cannot.  It must stay with the writer
task.  You are responsible for correctly invoking `reader_forked()` and `reader_joined()` to keep
the COW scope abreast of when the reader is active.  Failure to do so will lead to race conditions
or worse.

# Using the COW APIs as a reader

Import the `ReaderMethods` impl.  When you receive a handle, you can
invoke `h.read { |v| ... }` and so forth.  There is also a piece of
auxiliary data that can be optionally associated with each handle.

Note: if the type `T` contains mutable fields, then there is nothing
to stop the reader from mutating those fields in the `read()` method.
Do not do this.  It will lead to race conditions.

FIXME: We can enforce that this is not done by ensuring that the type
`T` contains no mutable fields.

# Auxiliary data

Readers can associate a piece of auxiliary data of type `A` along with
main nodes.  This is convenient but dangerous: it is the reader's job
to ensure that this data remains live independent of the COW nodes
themselves.

*/

use core::libc::types::os::arch::c95::size_t;
use ptr::Ptr;
use vec::push;

type ScopeData<T:Send,A> = {
    mut layout_active: bool,
    mut free_list: ~[Handle<T,A>],
    mut first_dirty: Handle<T,A>
};

struct ScopeResource<T:Send,A> {
    d : ScopeData<T,A>,

    drop unsafe {
        for self.d.free_list.each |h| { free_handle(*h); }
    }
}

fn ScopeResource<T:Send,A>(d : ScopeData<T,A>) -> ScopeResource<T,A> {
    ScopeResource { d: move d }
}

pub type Scope<T:Send,A> = @ScopeResource<T,A>;

type HandleData<T:Send,A> = {mut read_ptr: *T,
                             mut write_ptr: *mut T,
                             mut read_aux: *A,
                             mut next_dirty: Handle<T,A>};
pub enum Handle<T:Send,A> {
    _Handle(*HandleData<T,A>)
}

// Private methods
impl<T:Send,A> Handle<T,A> {
    fn read_ptr() -> *T unsafe            { (**self).read_ptr   }
    fn write_ptr() -> *mut T unsafe       { (**self).write_ptr  }
    fn read_aux() -> *A unsafe            { (**self).read_aux   }
    fn next_dirty() -> Handle<T,A> unsafe { (**self).next_dirty }

    fn set_read_ptr(t: *T) unsafe             { (**self).read_ptr = t;   }
    fn set_write_ptr(t: *mut T) unsafe        { (**self).write_ptr = t;  }
    fn set_read_aux(t: *A) unsafe             { (**self).read_aux = t;   }
    fn set_next_dirty(h: Handle<T,A>) unsafe { (**self).next_dirty = h; }

    pure fn is_null() -> bool { (*self).is_null() }
    fn is_not_null() -> bool { (*self).is_not_null() }
}

impl<T:Send,A> Handle<T,A> {
    /// Access the reader's view of the handle's data
    fn read<U>(f: fn(&T) -> U) -> U unsafe {
        f(&*self.read_ptr())
    }

    /// True if auxiliary data is associated with this handle
    fn has_aux() -> bool unsafe {
        self.read_aux().is_not_null()
    }

    /** Set the auxiliary data associated with this handle.

    **Warning:** the reader is responsible for keeping this data live!
    */
    fn set_aux(p: @A) unsafe {
        (**self).read_aux = ptr::to_unsafe_ptr(&*p);
    }

    /// Access the auxiliary data associated with this handle
    fn aux<U>(f: fn(&A) -> U) -> U unsafe {
        assert self.has_aux();
        f(&*self.read_aux())
    }
}

impl <T: Send,A> Handle<T,A> : cmp::Eq {
    pure fn eq(other: &Handle<T,A>) -> bool { *self == **other }
    pure fn ne(other: &Handle<T,A>) -> bool { *self != **other }
}

// Private methods
impl<T: Copy Send,A> Scope<T,A> {
    fn clone(v: *T) -> *T unsafe {
        let n: *mut T =
            cast::reinterpret_cast(&libc::calloc(sys::size_of::<T>() as size_t, 1u as size_t));

        // n.b.: this assignment will run the drop glue for <T,A>. *Hopefully* the fact that
        // everything is initialized to NULL by calloc will make this ok.  We may have to make the
        // take glue be tolerant of this.
        *n = unsafe{*v};

        return cast::reinterpret_cast(&n);
    }
}

unsafe fn free<T:Send>(t: *T) {
    let _x = move *cast::reinterpret_cast::<*T,*mut T>(&t);
    libc::free(cast::reinterpret_cast(&t));
}

unsafe fn free_handle<T:Send,A>(h: Handle<T,A>) {
    free(h.read_ptr());
    if h.write_ptr() != cast::reinterpret_cast(&h.read_ptr()) {
        free(cast::reinterpret_cast::<*mut T,*T>(&h.write_ptr()));
    }
}

pub unsafe fn unwrap<T:Send, A>(handle: Handle<T,A>) -> *HandleData<T,A> {
    *handle
}

pub unsafe fn wrap<T:Send, A>(data: *HandleData<T,A>) -> Handle<T,A> {
    _Handle(data)
}

fn null_handle<T:Send,A>() -> Handle<T,A> {
    _Handle(ptr::null())
}

pub fn Scope<T:Send,A>() -> Scope<T,A> {
    @ScopeResource({mut layout_active: false,
                    mut free_list: ~[],
                    mut first_dirty: null_handle()})
}

// Writer methods
impl<T:Copy Send,A> Scope<T,A> {
    fn is_reader_forked() -> bool {
        self.d.layout_active
    }

    fn reader_forked() {
        assert !self.d.layout_active;
        assert self.d.first_dirty.is_null();
        self.d.layout_active = true;
    }

    fn reader_joined() unsafe {
        assert self.d.layout_active;

        if self.d.first_dirty.is_not_null() {
            let mut handle = self.d.first_dirty;
            while (*handle).is_not_null() {
                free(handle.read_ptr());

                handle.set_read_ptr(cast::reinterpret_cast(&handle.write_ptr()));
                let next_handle = handle.next_dirty();
                handle.set_next_dirty(null_handle());
                handle = next_handle;
            }
            self.d.first_dirty = null_handle();
        }

        assert self.d.first_dirty.is_null();
        self.d.layout_active = false;
    }

    fn read<U>(h: &Handle<T,A>, f: fn(&T) -> U) -> U unsafe {
        // Use the write_ptr, which may be more up to date than the read_ptr or may not
        f(&*h.write_ptr())
    }

    fn write<U>(h: &Handle<T,A>, f: fn(&T) -> U) -> U unsafe {
        let const_read_ptr = ptr::const_offset(h.read_ptr(), 0);
        let const_write_ptr = ptr::const_offset(h.write_ptr(), 0);
        if self.d.layout_active && const_read_ptr == const_write_ptr {
            #debug["marking handle %? as dirty", h];
            h.set_write_ptr(cast::reinterpret_cast(&self.clone(h.read_ptr())));
            h.set_next_dirty(self.d.first_dirty);
            self.d.first_dirty = *h;
        }
        f(&*h.write_ptr())
    }

    // FIXME: This could avoid a deep copy by taking ownership of `v`
    #[allow(non_implicitly_copyable_typarams)]
    fn handle(v: &T) -> Handle<T,A> unsafe {
        debug!("vv: %?", *v);
        let d: *HandleData<T,A> =
            cast::reinterpret_cast(
                &libc::malloc(sys::size_of::<HandleData<T,A>>() as size_t));
        (*d).read_ptr = self.clone(ptr::to_unsafe_ptr(v));
        (*d).write_ptr = cast::reinterpret_cast(&(*d).read_ptr);
        (*d).read_aux = ptr::null();
        (*d).next_dirty = null_handle();
        let h = _Handle(d);
        push(&mut self.d.free_list, h);
        do self.read(&h) |v| {
            debug!("vv: %?", *v);
        }
        return h;
    }
}

#[cfg(test)]
#[allow(non_implicitly_copyable_typarams)]
mod test {
    type animal = {name: ~str, species: species};
    enum species {
        chicken(~chicken),
        bull(~bull)
    }
    type chicken = {mut eggs_per_day:uint};
    type bull = {mut horns:uint};

    type processed = {flag: bool};

    type animal_scope = Scope<animal, processed>;

    #[test]
    fn handles_get_freed() {
        let s: animal_scope = Scope();
        s.handle(&{name:~"henrietta", species:chicken(~{mut eggs_per_day:22u})});
        s.handle(&{name:~"ferdinand", species:bull(~{mut horns:3u})});
    }

    fn mutate(a: &animal) {
        match a.species {
          chicken(c) => c.eggs_per_day += 1u,
          bull(c) => c.horns += 1u
        }
    }

    fn read_characteristic(a: &animal) -> uint {
        match a.species {
          chicken(c) => c.eggs_per_day,
          bull(c) => c.horns
        }
    }

    #[test]
    fn interspersed_execution() {
        let s: animal_scope = Scope();
        let henrietta =
            s.handle(&{name:~"henrietta",
                      species:chicken(~{mut eggs_per_day:0u})});
        let ferdinand =
            s.handle(&{name:~"ferdinand",
                      species:bull(~{mut horns:0u})});

        let iter1 = 3u;
        let iter2 = 22u;
        let read_port = comm::Port();
        let read_chan = comm::Chan(&read_port);

        // fire up a reader task
        for uint::range(0u, iter1) |i| {
            s.reader_forked();
            let wait_chan = task::spawn_listener(|wait_port| {
                for uint::range(0u, iter2) |_i| {
                    comm::send(read_chan, henrietta.read(read_characteristic));
                    comm::send(read_chan, ferdinand.read(read_characteristic));
                    comm::recv(wait_port);
                }
            });

            let hrc = henrietta.read(read_characteristic);
            assert hrc == (i * iter2);

            let frc = ferdinand.read(read_characteristic);
            assert frc == i * iter2;

            for uint::range(0u, iter2) |_i| {
                assert hrc == comm::recv(read_port);
                s.write(&henrietta, mutate);
                assert frc == comm::recv(read_port);
                s.write(&ferdinand, mutate);
                comm::send(wait_chan, ());
            }
            s.reader_joined();
        }

        assert henrietta.read(read_characteristic) == iter1 * iter2;
        assert ferdinand.read(read_characteristic) == iter1 * iter2;
    }
}
