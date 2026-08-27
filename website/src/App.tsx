import { Cta, Footer } from './components/Footer'
import { Features } from './components/Features'
import { Hero } from './components/Hero'
import { Nav } from './components/Nav'
import { Reference } from './components/Reference'
import { Showcase } from './components/Showcase'

export function App() {
  return (
    <div id="top">
      <Nav />
      <main>
        <Hero />
        <Features />
        <Showcase />
        <Reference />
        <Cta />
      </main>
      <Footer />
    </div>
  )
}
