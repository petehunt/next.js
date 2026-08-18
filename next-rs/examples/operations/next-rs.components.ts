// The registry from spec §24. Only components listed here may be referenced from
// Rust, and every one of them must be a Client Component.
import Account from '@/components/Account'
import Metrics from '@/components/Metrics'
import Notifications from '@/components/Notifications'

export default {
  Account,
  Metrics,
  Notifications,
}
