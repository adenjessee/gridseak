pub trait Area {
    fn area(&self) -> f64;
    fn twice(&self) -> f64 {
        self.area() * 2.0
    }
}

pub struct Circle(pub f64);
pub struct Square(pub f64);

impl Circle {
    pub fn new(r: f64) -> Self {
        Circle(r)
    }
    pub fn is_empty(&self) -> bool {
        self.0 == 0.0
    }
}

impl Square {
    pub fn is_empty(&self) -> bool {
        self.0 == 0.0
    }
}

impl Area for Circle {
    fn area(&self) -> f64 {
        3.0 * self.0 * self.0
    }
}

impl Area for Square {
    fn area(&self) -> f64 {
        self.0 * self.0
    }
}
