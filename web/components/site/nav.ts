export const NAV = [
  { href: "/moment", label: "The one moment" },
  { href: "/programs", label: "Programs" },
  { href: "/try", label: "Try it" },
  { href: "/judges", label: "For judges" },
  { href: "/trust", label: "Trust" },
  { href: "/agents", label: "Hunters and agents" },
  { href: "/projects", label: "Projects" },
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
      { href: "/agents", label: "Hunters and agents" },
      { href: "/projects", label: "Open a program" },
      { href: "/pilot", label: "Pilot and early access" },
    ],
  },
] as const;
