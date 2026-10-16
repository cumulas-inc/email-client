import { render, screen } from "@testing-library/react";
import { beforeAll, describe, expect, it } from "vitest";

import { setLocaleForTesting } from "../../lib/datetime";
import { FileName } from "./Attachments";
import { Count } from "./Count";

beforeAll(() => setLocaleForTesting("en-US"));

describe("Count", () => {
  it("groups thousands and puts the exact number in a tooltip when shortened", () => {
    const { rerender } = render(<Count n={1204} />);
    expect(screen.getByText("1,204")).not.toHaveAttribute("title");
    rerender(<Count n={12_345} />);
    expect(screen.getByText("12K")).toHaveAttribute("title", "12,345");
  });
});

describe("FileName", () => {
  it("shows the end of a long name apart, so it survives the cut", () => {
    const name = "Northwind_Industries_Holdings_MSA_v14_consolidated_redlines_FINAL_final_v2.docx";
    const { container } = render(<FileName name={name} />);
    const root = container.firstElementChild!;
    expect(root).toHaveAttribute("title", name);
    expect(root).toHaveAttribute("dir", "auto");
    expect(root).toHaveTextContent(name);
    expect(container.querySelector(".mail-filename__tail")).toHaveTextContent("L_final_v2.docx");
  });

  it("keeps a short name in one piece", () => {
    render(<FileName name="invoice.pdf" />);
    expect(screen.getByText("invoice.pdf")).toHaveClass("mail-filename__head");
  });
});
