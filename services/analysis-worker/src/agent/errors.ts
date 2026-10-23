export type PlanningErrorCode =
  | "cancelled"
  | "invalid_input"
  | "invalid_limits"
  | "model_not_configured"
  | "model_not_found"
  | "persistence_failed";

/** Stable, safe configuration/input failures. Never include repository text. */
export class PlanningError extends Error {
  readonly code: PlanningErrorCode;

  constructor(code: PlanningErrorCode, message: string) {
    super(message);
    this.name = "PlanningError";
    this.code = code;
  }
}

/** Tool failures are returned to the model as structured error results. */
export class PlanningToolError extends Error {
  readonly code:
    | "invalid_argument"
    | "not_found"
    | "not_allowed"
    | "output_too_large"
    | "limit_exceeded";

  constructor(code: PlanningToolError["code"], message: string) {
    super(message);
    this.name = "PlanningToolError";
    this.code = code;
  }
}
