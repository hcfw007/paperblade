export type ToolId =
  | "merge"
  | "split"
  | "compress"
  | "encrypt"
  | "watermark"
  | "convert";

export type Tool = {
  id: ToolId;
  title: string;
  subtitle: string;
  href: string;
  available: boolean;
};

export const TOOLS: Tool[] = [
  {
    id: "merge",
    title: "Merge",
    subtitle: "Combine multiple PDFs into one",
    href: "/merge",
    available: true,
  },
  {
    id: "split",
    title: "Split",
    subtitle: "Break a PDF into separate files",
    href: "/split",
    available: true,
  },
  {
    id: "compress",
    title: "Compress",
    subtitle: "Shrink file size with quality presets",
    href: "/compress",
    available: false,
  },
  {
    id: "encrypt",
    title: "Encrypt & Decrypt",
    subtitle: "Add or remove a PDF password",
    href: "/encrypt",
    available: false,
  },
  {
    id: "watermark",
    title: "Watermark",
    subtitle: "Stamp text or images on every page",
    href: "/watermark",
    available: false,
  },
  {
    id: "convert",
    title: "Convert",
    subtitle: "PDF to images and images to PDF",
    href: "/convert",
    available: false,
  },
];
