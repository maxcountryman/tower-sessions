// Not sure how this should look like, Maybe only the `expires` method is enough.
pub trait Expires {
    type Expiry;

    fn expires(&self) -> Self::Expiry;

    fn set_expires(&mut self, expiry: Self::Expiry);

    fn expired(&self) -> bool;
}

pub struct NoExpiry<T>(pub T);

impl Expires for NoExpiry<()> {
    type Expiry = ();

    fn expires(&self) {}

    fn set_expires(&mut self, _: ()) {}

    fn expired(&self) -> bool {
        false
    }
}
