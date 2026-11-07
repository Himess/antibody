import Link from "next/link";

/** The persistent, quiet notice on every page (README: Safety). */
export function TestnetNotice() {
  return (
    <div className="notice" role="note" aria-label="Testnet notice">
      <div className="page notice-inner">
        <span className="notice-dot" aria-hidden />
        <span>
          <strong>Testnet.</strong> wNEAR has no value. Acme Lend and the practice vaults are our own deliberately
          vulnerable fixtures. <Link href="/judges#limits">Limits</Link>
        </span>
      </div>
    </div>
  );
}
