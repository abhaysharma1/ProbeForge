import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("App", () => {
  it("renders the product name and both job goals", () => {
    render(<App />);

    expect(screen.getByRole("heading", { level: 1, name: "ProbeForge" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Vulnerability testing" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Load testing" })).toBeInTheDocument();
  });

  it("states the approval and authorization boundary", () => {
    render(<App />);

    expect(
      screen.getByText(/explicit plan approval and target authorization/i),
    ).toBeInTheDocument();
  });
});
