import { SiteFooter } from "@/components/site/SiteFooter";
import { SiteHeader } from "@/components/site/SiteHeader";
import { TestnetNotice } from "@/components/site/TestnetNotice";

export default function SiteLayout({ children }: { children: React.ReactNode }) {
  return (
    <>
      <TestnetNotice />
      <SiteHeader />
      <main id="main">{children}</main>
      <SiteFooter />
    </>
  );
}
