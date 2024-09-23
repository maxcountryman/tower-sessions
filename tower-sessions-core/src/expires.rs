pub trait Expires {
    fn expired(&self) -> bool;
}

pub struct NoExpiry<T>(pub T);

impl Expires for NoExpiry<()> {
    fn expired(&self) -> bool {
        false
    }
}
