// The component registry from spec §24. Only components listed here may be
// referenced from Rust, and every one of them must be a Client Component.
//
// `blog-starter` has sixteen presentational components and one Client Component.
// Only components that need React are here; the presentational ones became Rust
// string templates in `src/views.rs`.
import SubscribeForm from '@/components/SubscribeForm'
import ThemeSwitcher from '@/components/ThemeSwitcher'

export default {
  SubscribeForm,
  ThemeSwitcher,
}
