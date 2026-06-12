// Wire types for the HitlCard primitive. Kept in a sibling file so
// `lib/mcp-tools.ts` can import the type without pulling in JSX.

export type HitlCardKind = "constructive" | "diff" | "destructive"

export type HitlCardState =
  | "pending"
  | "committing"
  | "committed"
  | "rejected"
  | "failed"
  | "superseded"

export interface HitlFieldSpec {
  /** Human-readable field label. */
  label: string
  /** Initial value rendered in the field. */
  value: string
  /** Whether the user can edit this field before commit. */
  editable: boolean
  /**
   * Key the edited value lives under in the card's edit state. Required
   * when `editable: true`. The chat layer reads `editValues[key]` when
   * the user commits.
   */
  key?: string
  /** Optional placeholder for editable fields. */
  placeholder?: string
}

export interface HitlConstructiveBody {
  fields: HitlFieldSpec[]
}

export interface HitlDiffBody {
  before: Record<string, string>
  after: Record<string, string>
}

export interface HitlDestructiveBody {
  info: string
}

export type HitlCardBody =
  | HitlConstructiveBody
  | HitlDiffBody
  | HitlDestructiveBody

export interface HitlCommitButton {
  label: string
  intent: "primary" | "destructive"
}

export interface HitlRejectButton {
  label: string
}

export interface HitlTypeToConfirm {
  promptLabel: string
  expectedValue: string
}

export interface HitlCard {
  kind: HitlCardKind
  title: string
  summary?: string
  body: HitlCardBody
  sideEffects?: string[]
  reversibility?: string
  commit: HitlCommitButton
  reject: HitlRejectButton
  confirmTyping?: HitlTypeToConfirm
}
