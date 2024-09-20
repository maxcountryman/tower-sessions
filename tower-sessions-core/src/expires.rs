// Not sure how this should look like, Maybe only the `expires` method is enough.
pub trait Expires {
    type Expiry/*: What type of trait should this be bounded by? */;

    fn expires(&self) -> Self::Expiry;

    fn expired(&self) -> bool;
}

pub struct NoExpiry<T>(pub T);

impl Expires for NoExpiry<()> {
    type Expiry = ();

    fn expires(&self) {}

    fn expired(&self) -> bool {
        false
    }
}
