mod and;
mod and_then;
mod boxed;
mod map;
mod map_err;
mod or;
mod or_else;
mod recover;
mod service;
mod unify;
mod unit;
mod wrap;

use futures::{future, Future, IntoFuture};

pub(crate) use ::generic::{Combine, Either, Func, HList, One, one, Tuple};
use ::reject::{CombineRejection, Reject, Rejection};
use ::route::{self, Route};

pub(crate) use self::and::And;
use self::and_then::AndThen;
pub use self::boxed::BoxedFilter;
pub(crate) use self::map::Map;
pub(crate) use self::map_err::MapErr;
pub(crate) use self::or::Or;
use self::or_else::OrElse;
use self::recover::Recover;
use self::unify::Unify;
use self::unit::Unit;
pub(crate) use self::wrap::{WrapSealed, Wrap};

// A crate-private base trait, allowing the actual `filter` method to change
// signatures without it being a breaking change.
pub trait FilterBase {
    type Extract: Tuple; // + Send;
    type Error: Reject;
    type Future: Future<Item=Self::Extract, Error=Self::Error> + Send;

    fn filter(&self) -> Self::Future;

    // crate-private for now

    fn map_err<F, E>(self, fun: F) -> MapErr<Self, F>
    where
        Self: Sized,
        F: Fn(Self::Error) -> E + Clone,
        E: ::std::fmt::Debug + Send,
    {
        MapErr {
            filter: self,
            callback: fun,
        }
    }

    fn unit(self) -> Unit<Self>
    where
        Self: Filter<Extract=((),)> + Sized,
    {
        Unit {
            filter: self,
        }
    }
}

/// This just makes use of rustdoc's ability to make compile_fail tests.
/// This is specifically testing to make sure `Filter::filter` isn't
/// able to be called from outside the crate (since rustdoc tests are
/// compiled as new crates).
///
/// ```compile_fail
/// use warp::Filter;
///
/// let _ = warp::any().filter();
/// ```
pub fn __warp_filter_compilefail_doctest() {
    // Duplicate code to make sure the code is otherwise valid.
    let _ = ::any().filter();
}

/// Composable request filters.
///
/// A `Filter` can optionally extract some data from a request, combine
/// it with others, mutate it, and return back some value as a reply. The
/// power of `Filter`s come from being able to isolate small subsets, and then
/// chain and reuse them in various parts of your app.
///
/// # Extracting Tuples
///
/// You may notice that several of these filters extract some tuple, often
/// times a tuple of just 1 item! Why?
///
/// If a filter extracts a `(String,)`, that simply means that it
/// extracts a `String`. If you were to `map` the filter, the argument type
/// would be exactly that, just a `String`.
///
/// What is it? It's just some type magic that allows for automatic combining
/// and flattening of tuples. Without it, combining two filters together with
/// `and`, where one extracted `()`, and another `String`, would mean the
/// `map` would be given a single argument of `((), String,)`, which is just
/// no fun.
pub trait Filter: FilterBase {
    /// Composes a new `Filter` that requires both this and the other to filter a request.
    ///
    /// Additionally, this will join together the extracted values of both
    /// filters, so that `map` and `and_then` receive them as separate arguments.
    ///
    /// If a `Filter` extracts nothing (so, `()`), combining with any other
    /// filter will simply discard the `()`. If a `Filter` extracts one or
    /// more items, combining will mean it extracts the values of itself
    /// combined with the other.
    ///
    /// # Example
    ///
    /// ```
    /// use warp::Filter;
    ///
    /// // Match `/hello/:name`...
    /// warp::path("hello")
    ///     .and(warp::path::param::<String>());
    /// ```
    fn and<F>(self, other: F) -> And<Self, F>
    where
        Self: Sized,
        //Self::Extract: HList + Combine<F::Extract>,
        <Self::Extract as Tuple>::HList: Combine<<F::Extract as Tuple>::HList>,
        F: Filter + Clone,
        F::Error: CombineRejection<Self::Error>,
    {
        And {
            first: self,
            second: other,
        }
    }

    /// Composes a new `Filter` of either this or the other filter.
    ///
    /// # Example
    ///
    /// ```
    /// use std::net::SocketAddr;
    /// use warp::Filter;
    ///
    /// // Match either `/:u32` or `/:socketaddr`
    /// warp::path::param::<u32>()
    ///     .or(warp::path::param::<SocketAddr>());
    /// ```
    fn or<F>(self, other: F) -> Or<Self, F>
    where
        Self: Sized,
        F: Filter,
        F::Error: CombineRejection<Self::Error>,
    {
        Or {
            first: self,
            second: other,
        }
    }

    /// Composes this `Filter` with a function receiving the extracted value.
    ///
    ///
    /// # Example
    ///
    /// ```
    /// use warp::Filter;
    ///
    /// // Map `/:id`
    /// warp::path::param().map(|id: u64| {
    ///   format!("Hello #{}", id)
    /// });
    /// ```
    ///
    /// # `Func`
    ///
    /// The generic `Func` trait is implemented for any function that receives
    /// the same arguments as this `Filter` extracts. In practice, this
    /// shouldn't ever bother you, and simply makes things feel more natural.
    ///
    /// For example, if three `Filter`s were combined together, suppose one
    /// extracts nothing (so `()`), and the other two extract two integers,
    /// a function that accepts exactly two integer arguments is allowed.
    /// Specifically, any `Fn(u32, u32)`.
    ///
    /// Without `Product` and `Func`, this would be a lot messier. First of
    /// all, the `()`s couldn't be discarded, and the tuples would be nested.
    /// So, instead, you'd need to pass an `Fn(((), (u32, u32)))`. That's just
    /// a single argument. Bleck!
    ///
    /// Even worse, the tuples would shuffle the types around depending on
    /// the exact invocation of `and`s. So, `unit.and(int).and(int)` would
    /// result in a different extracted type from `unit.and(int.and(int)`,
    /// or from `int.and(unit).and(int)`. If you changed around the order
    /// of filters, while still having them be semantically equivalent, you'd
    /// need to update all your `map`s as well.
    ///
    /// `Product`, `HList`, and `Func` do all the heavy work so that none of
    /// this is a bother to you. What's more, the types are enforced at
    /// compile-time, and tuple flattening is optimized away to nothing by
    /// LLVM.
    fn map<F>(self, fun: F) -> Map<Self, F>
    where
        Self: Sized,
        F: Func<Self::Extract> + Clone,
    {
        Map {
            filter: self,
            callback: fun,
        }
    }


    /// Composes this `Filter` with a function receiving the extracted value.
    ///
    /// The function should return some `IntoFuture` type.
    ///
    /// # Example
    ///
    /// ```
    /// use warp::Filter;
    ///
    /// // Validate after `/:id`
    /// warp::path::param().and_then(|id: u64| {
    ///     if id != 0 {
    ///         Ok(format!("Hello #{}", id))
    ///     } else {
    ///         Err(warp::reject())
    ///     }
    /// });
    /// ```
    fn and_then<F>(self, fun: F) -> AndThen<Self, F>
    where
        Self: Sized,
        F: Func<Self::Extract> + Clone,
        F::Output: IntoFuture + Send,
        <F::Output as IntoFuture>::Error: CombineRejection<Self::Error>,
        <F::Output as IntoFuture>::Future: Send,
    {
        AndThen {
            filter: self,
            callback: fun,
        }
    }

    /// Compose this `Filter` with a function receiving an error.
    ///
    /// The function should return some `IntoFuture` type yielding the
    /// same item and error types.
    fn or_else<F>(self, fun: F) -> OrElse<Self, F>
    where
        Self: Sized,
        F: Func<Self::Error>,
        F::Output: IntoFuture<Item=Self::Extract, Error=Self::Error> + Send,
        <F::Output as IntoFuture>::Future: Send,
    {
        OrElse {
            filter: self,
            callback: fun,
        }
    }

    /// Compose this `Filter` with a function receiving an error and
    /// returning a *new* type, instead of the *same* type.
    ///
    /// This is useful for "customizing" rejections into new response types.
    /// See also the [errors example][ex].
    ///
    /// [ex]: https://github.com/seanmonstar/warp/blob/master/examples/errors.rs
    fn recover<F>(self, fun: F) -> Recover<Self, F>
    where
        Self: Sized,
        F: Func<Self::Error>,
        F::Output: IntoFuture<Error=Self::Error> + Send,
        <F::Output as IntoFuture>::Future: Send,
    {
        Recover {
            filter: self,
            callback: fun,
        }
    }

    /// Unifies the extracted value of `Filter`s composed with `or`.
    ///
    /// When a `Filter` extracts some `Either<T, T>`, where both sides
    /// are the same type, this combinator can be used to grab the
    /// inner value, regardless of which side of `Either` it was. This
    /// is useful for values that could be extracted from multiple parts
    /// of a request, and the exact place isn't important.
    ///
    /// # Example
    ///
    /// ```rust
    /// use std::net::SocketAddr;
    /// use warp::Filter;
    ///
    /// let client_ip = warp::header("x-real-ip")
    ///     .or(warp::header("x-forwarded-for"))
    ///     .unify()
    ///     .map(|ip: SocketAddr| {
    ///         // Get the IP from either header,
    ///         // and unify into the inner type.
    ///     });
    /// ```
    fn unify<T>(self) -> Unify<Self>
    where
        Self: Filter<Extract=(Either<T, T>,)> + Sized,
        T: Tuple,
    {
        Unify {
            filter: self,
        }
    }

    /// Wraps the current filter with some wrapper.
    ///
    /// The wrapper may do some preparation work before starting this filter,
    /// and may do post-processing after the filter completes.
    ///
    /// # Example
    ///
    /// ```
    /// use warp::Filter;
    ///
    /// let route = warp::any()
    ///     .map(warp::reply);
    ///
    /// // Wrap the route with a log wrapper.
    /// let route = route.with(warp::log("example"));
    /// ```
    fn with<W>(self, wrapper: W) -> W::Wrapped
    where
        Self: Sized,
        W: Wrap<Self>,
    {
        wrapper.wrap(self)
    }

    /// Boxes this filter into a trait object, making it easier to name the type.
    ///
    /// # Example
    ///
    /// ```
    /// use warp::Filter;
    ///
    /// fn impl_reply() -> warp::filters::BoxedFilter<(impl warp::Reply,)> {
    ///     warp::any()
    ///         .map(warp::reply)
    ///         .boxed()
    /// }
    ///
    /// fn named_i32() -> warp::filters::BoxedFilter<(i32,)> {
    ///     warp::path::param::<i32>()
    ///         .boxed()
    /// }
    ///
    /// fn named_and() -> warp::filters::BoxedFilter<(i32, String)> {
    ///     warp::path::param::<i32>()
    ///         .and(warp::header::<String>("host"))
    ///         .boxed()
    /// }
    /// ```
    fn boxed(self) -> BoxedFilter<Self::Extract>
    where
        Self: Sized + Send + Sync + 'static,
        Self::Extract: Send,
        Rejection: From<Self::Error>,
    {
        BoxedFilter::new(self)
    }
}

impl<T: FilterBase> Filter for T {}

pub trait FilterClone: Filter + Clone {}

impl<T: Filter + Clone> FilterClone for T {}

fn _assert_object_safe() {
    fn _assert(_f: &Filter<
        Extract=(),
        Error=(),
        Future=future::FutureResult<(), ()>
    >) {}
}

// ===== FilterFn =====

pub(crate) fn filter_fn<F, U>(func: F) -> FilterFn<F>
where
    F: Fn(&mut Route) -> U,
    U: IntoFuture,
    U::Item: Tuple,
    U::Error: Reject,
{
    FilterFn {
        func,
    }
}

pub(crate) fn filter_fn_one<F, U>(func: F)
    -> FilterFn<impl Fn(&mut Route) -> future::Map<U::Future, fn(U::Item) -> (U::Item,)> + Copy>
where
    F: Fn(&mut Route) -> U + Copy,
    U: IntoFuture,
    U::Error: Reject,
{
    filter_fn(move |route| {
        func(route)
            .into_future()
            .map(tup_one as _)
    })
}

fn tup_one<T>(item: T) -> (T,) {
    (item,)
}

#[derive(Copy, Clone)]
#[allow(missing_debug_implementations)]
pub(crate) struct FilterFn<F> {
    // TODO: could include a `debug_str: &'static str` to be used in Debug impl
    func: F,
}

impl<F, U> FilterBase for FilterFn<F>
where
    F: Fn(&mut Route) -> U,
    U: IntoFuture,
    U::Future: Send,
    U::Item: Tuple,
    U::Error: Reject,
{
    type Extract = U::Item;
    type Error = U::Error;
    type Future = U::Future;

    #[inline]
    fn filter(&self) -> Self::Future {
        route::with(|route| {
            (self.func)(route).into_future()
        })
    }
}

