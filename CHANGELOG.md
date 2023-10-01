# Unreleased

# 0.2.0

**Breaking Changes**

- Renamed store error variants for consistency (SqlxStoreError, RedisStoreError). #18
- Moved MySQL `expiration_time` column to `timestamp(6), for microsecond resolution. #14
- Replaced `Session.with_max_age` with `set_expiration_time` and `set_expiration_time_from_max_age`, allowing applications to control session durations dynamically. #7

**Other changes**

- Provide layered caching via `CachingSessionStore` #8
- Provide a Moka store #6 (Thank you @and-reas-se!)
- Provide a MongoDB store #5 (Thank you @JustMangoT!)

# 0.1.0

- Initial release :tada:
