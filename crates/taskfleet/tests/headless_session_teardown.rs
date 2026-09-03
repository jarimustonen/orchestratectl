//! Headless session teardown is covered by the typed supervisor cleanup unit
//! tests (`managed_session_*`) and the native end-to-end spinoff round trip.
//! The former create-script integration fixture was removed with that production
//! backend; native spawn integration uses `NativeSpawnTools` in sibling suites.
