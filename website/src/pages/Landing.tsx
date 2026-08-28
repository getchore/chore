import { Cta } from '../components/Footer'
import { Dogfood } from '../components/Dogfood'
import { Features } from '../components/Features'
import { Hero } from '../components/Hero'
import { Reference } from '../components/Reference'
import { Replaces } from '../components/Replaces'
import { Showcase } from '../components/Showcase'

export function Landing() {
  return (
    <>
      <Hero />
      <Features />
      <Replaces />
      <Showcase />
      <Dogfood />
      <Reference />
      <Cta />
    </>
  )
}
