/** Pages with no site chrome (the presentation mode). */
export default function BareLayout({ children }: { children: React.ReactNode }) {
  return <main id="main">{children}</main>;
}
