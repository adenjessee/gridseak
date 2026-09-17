pub fn callee() -> i32 {
    1
}

pub fn caller() -> i32 {
    callee()
}
