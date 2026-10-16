import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Count } from "./Count";

describe("Count", () => {
  it("groups thousands and shortens large counts, with the exact number as a tooltip", () => {
    render(
      <>
        <Count n={7} />
        <Count n={1204} />
        <Count n={12345} />
      </>,
    );
    expect(screen.getByText("7")).not.toHaveAttribute("title");
    expect(screen.getByText("1,204")).not.toHaveAttribute("title");
    expect(screen.getByText("12K")).toHaveAttribute("title", "12,345");
  });
});
