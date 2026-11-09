export const NAV = [
  { href: "/moment", label: "The moment" },
  { href: "/programs", label: "Programs" },
  { href: "/try", label: "Try it" },
  { href: "/judges", label: "For judges" },
  { href: "/trust", label: "Trust" },
  { href: "/agents", label: "Hunters & agents" },
  { href: "/projects", label: "For protocols" },
] as const;

export const FOOTER = [
  {
    title: "Product",
    links: [
      { href: "/moment", label: "The one moment" },
      { href: "/programs", label: "Programs" },
      { href: "/try", label: "Try it, no account" },
      { href: "/account", label: "Your account" },
    ],
  },
  {
    title: "Verify",
    links: [
      { href: "/judges", label: "For judges" },
      { href: "/trust", label: "Trust" },
      { href: "/status", label: "Status" },
      { href: "/mainnet", label: "How mainnet works" },
    ],
  },
  {
    title: "Build",
    links: [
      { href: "/agents", label: "Hunters & agents" },
      { href: "/projects", label: "For protocols" },
      { href: "/pilot", label: "Pilot and early access" },
    ],
  },
] as const;
