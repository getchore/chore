import { Cta, Footer } from './components/Footer'
import { Dogfood } from './components/Dogfood'
import { Features } from './components/Features'
import { Hero } from './components/Hero'
import { Nav } from './components/Nav'
import { Reference } from './components/Reference'
import { Replaces } from './components/Replaces'
import { Showcase } from './components/Showcase'
import { useHashScroll } from './lib/useHashScroll'

export function App() {
  useHashScroll()

  return (
    <div id="top">
      <Nav />
      <main>
        <Hero />
        <Features />
        <Replaces />
        <Showcase />
        <Dogfood />
        <Reference />
        <Cta />
      </main>
      <Footer />
    </div>
  )
}
