pub mod shapes;
use shapes::{Area, Circle, Square};

macro_rules! twice {
    ($e:expr) => {{
        $e;
        $e
    }};
}

macro_rules! call_helper {
    () => {
        helper(9)
    };
}

pub fn helper(x: u32) -> u32 {
    x + 1
}

pub fn via_user_macro() -> u32 {
    call_helper!()
}

pub fn turbofish() -> u32 {
    generic::<u32>(5)
}

pub fn generic<T: Into<u32>>(x: T) -> u32 {
    x.into()
}

pub fn twice_dyn(s: &dyn Area) -> f64 {
    s.twice()
}

pub fn direct() -> u32 {
    helper(1)
}

pub fn method_call() -> f64 {
    let c = Circle::new(1.0);
    c.area()
}

pub fn ufcs() -> f64 {
    let c = Circle::new(2.0);
    Circle::area(&c) + <Square as Area>::area(&Square(1.0))
}

pub fn dyn_dispatch(s: &dyn Area) -> f64 {
    s.area()
}

pub fn generic_bound<T: Area>(s: &T) -> f64 {
    s.area()
}

pub fn in_closure() -> u32 {
    let f = |y: u32| helper(y);
    f(2)
}

pub fn in_macro() -> String {
    twice!(helper(3));
    let v = vec![helper(4)];
    format!("{}", helper(v[0]))
}

pub fn value_use() -> fn(u32) -> u32 {
    helper
}

pub fn iter_map() -> Vec<u32> {
    vec![1, 2].into_iter().map(helper).collect()
}

pub fn empties(c: &Circle, s: &Square) -> bool {
    c.is_empty() && s.is_empty()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub x: i32,
}

pub fn uses_derive(p: &Point) -> bool {
    let q = p.clone();
    q == *p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_direct() {
        assert_eq!(direct(), 2);
    }
}
