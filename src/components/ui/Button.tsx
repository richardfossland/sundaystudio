import { cva, type VariantProps } from "class-variance-authority";
import type { ButtonHTMLAttributes } from "react";

import { cn } from "@/lib/cn";

const button = cva(
  "inline-flex items-center justify-center gap-2 rounded-[var(--radius-md)] font-medium transition-colors duration-[var(--duration-fast)] disabled:pointer-events-none disabled:opacity-50 focus-visible:outline-2 focus-visible:outline-[var(--color-accent)]",
  {
    variants: {
      variant: {
        accent:
          "bg-[var(--color-accent)] text-[var(--color-accent-fg)] hover:bg-[var(--color-gold-500)]",
        surface:
          "bg-[var(--color-bg-surface)] text-[var(--color-fg)] hover:bg-[var(--color-neutral-700)]",
        ghost:
          "bg-transparent text-[var(--color-fg-muted)] hover:bg-[var(--color-bg-surface)] hover:text-[var(--color-fg)]",
      },
      size: {
        sm: "h-8 px-3 text-ui-sm",
        md: "h-10 px-4 text-ui-sm",
        lg: "h-12 px-6 text-ui-md",
      },
    },
    defaultVariants: { variant: "surface", size: "md" },
  },
);

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof button>;

export function Button({ className, variant, size, ...props }: ButtonProps) {
  return (
    <button className={cn(button({ variant, size }), className)} {...props} />
  );
}
