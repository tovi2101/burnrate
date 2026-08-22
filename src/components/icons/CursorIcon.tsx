import type { SVGProps } from "react";

export function CursorIcon(props: SVGProps<SVGSVGElement>) {
  return <svg viewBox="0 0 24 24" fill="none" aria-hidden="true" {...props}>
    <path d="m4.25 3.5 15.5 8.1-7.1 2.15-2.2 6.75L4.25 3.5Z" stroke="currentColor" strokeWidth="1.7" strokeLinejoin="round" />
    <path d="m10.7 12.7 4.8 4.8" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" />
  </svg>;
}
