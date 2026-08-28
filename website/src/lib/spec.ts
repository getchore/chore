import raw from '../spec.json'

/**
 * The shape of `chore spec`. Nothing here names a builtin, a variable or a
 * syntax form: the reference page is a renderer for this document, and the
 * only way it can describe a language the binary does not implement is if
 * the binary emits something wrong.
 *
 * See vite.config.ts — src/spec.json is regenerated from the chore binary at
 * build time and committed as a snapshot for machines that lack one.
 */
export type Flag = {
  name: string
  argument: string | null
  default: string | null
  meaning: string
}

export type Builtin = {
  name: string
  usage: string
  summary: string
  description: string
  /** True when the command touches the world, which is what `--dry` skips. */
  effects: boolean
  flags: Flag[]
}

export type Variable = {
  name: string
  values: string
  meaning: string
  /** "run" — fixed for the whole invocation; "task" — depends on where you are. */
  scope: string
}

export type SyntaxForm = { name: string; syntax: string; example: string; meaning: string }
export type Condition = { syntax: string; meaning: string }
export type Chain = { symbol: string; meaning: string }
export type Named = { name: string; rule: string }

export type Spec = {
  version: string
  builtins: Builtin[]
  variables: Variable[]
  syntax: SyntaxForm[]
  conditions: Condition[]
  chaining: Chain[]
  resolution: Named[]
  rules: Named[]
  reserved_tasks: string[]
  namespace_separator: string
}

export const spec = raw as unknown as Spec

/** Distinct `scope` values, in the order the spec lists them. */
export function scopes(): string[] {
  return [...new Set(spec.variables.map((v) => v.scope))]
}

export function variablesIn(scope: string): Variable[] {
  return spec.variables.filter((v) => v.scope === scope)
}
