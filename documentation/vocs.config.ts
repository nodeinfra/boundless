import biomePlugin from "vite-plugin-biome";
import VitePluginSitemap from "vite-plugin-sitemap";
import { defineConfig } from "vocs";

const SIDEBAR_ITEMS = [
  {
    text: "Introduction",
    collapsed: true,
    items: [
      {
        text: "What is Boundless?",
        link: "/developers/what",
      },
      {
        text: "Why use Boundless?",
        link: "/developers/why",
      },

    ],
  },
  {
    text: "Build",
    collapsed: true,
    items: [
      {
        text: "Quick Start",
        link: "/developers/quick-start",
      },
      {
        text: "Core Concepts",
        items: [
          {
            text: "Build a Program",
            link: "/developers/tutorials/build",
          },
          {
            text: "Proof Lifecycle",
            link: "/developers/proof-lifecycle",
          },
          {
            text: "Request a Proof",
            link: "/developers/tutorials/request",
          },
          {
            text: "Tracking your Request",
            link: "/developers/tutorials/tracking",
          },
          {
            text: "Pricing a Request",
            link: "/developers/tutorials/pricing",
          },
          {
            text: "Use a Proof",
            link: "/developers/tutorials/use",
          },
          {
            text: "Troubleshooting",
            link: "/developers/tutorials/troubleshooting",
          },
        ],
      },
      {
        text: "Tutorials",
        items: [
          {
            text: "Callbacks",
            link: "/developers/tutorials/callbacks",
          },
          {
            text: "Proof Composition",
            link: "/developers/tutorials/proof-composition",
          },
          {
            text: "Proof Types",
            link: "/developers/tutorials/proof-types",
          },
          {
            text: "Migrating from Bonsai",
            link: "/developers/tutorials/bonsai",
          },
          {
            text: "Sensitive Inputs",
            link: "/developers/tutorials/sensitive-inputs",
          },
          {
            text: "Smart Contract Requestors",
            link: "/developers/tutorials/smart-contract-requestor",
          },
        ],
      },
      {
        text: "Smart Contracts",
        items: [
          {
            text: "Boundless Contracts",
            link: "/developers/smart-contracts/reference",
          },
          {
            text: "Chains & Deployments",
            link: "/developers/smart-contracts/deployments",
          },
          {
            text: "Verifier Contracts",
            link: "/developers/smart-contracts/verifier-contracts",
          },
        ],
      },
      {
        text: "Dev Tooling",
        collapsed: true,
        items: [
          {
            text: "Boundless SDK",
            link: "/developers/tooling/sdk",
          },
          {
            text: "Boundless CLI",
            link: "/developers/tooling/cli",
          },
        ],
      },
      {
        text: "Steel",
        collapsed: true,
        link: "/developers/steel/quick-start",
        items: [
          {
            text: "Quick Start",
            link: "/developers/steel/quick-start",
          },
          {
            text: "What is Steel?",
            link: "/developers/steel/what-is-steel",
          },
          {
            text: "How does Steel work?",
            link: "/developers/steel/how-it-works",
          },
          {
            text: "Commitments",
            link: "/developers/steel/commitments",
          },
          {
            text: "History",
            link: "/developers/steel/history",
          },
          {
            text: "Events",
            link: "/developers/steel/events",
          },
          {
            text: "Crate Docs",
            link: "https://boundless-xyz.github.io/steel/risc0_steel/index.html",
          },
        ],
      },
      {
        text: "Kailua",
        collapsed: true,
        link: "/developers/kailua/intro",
        items: [
          {
            text: "Introducing Kailua",
            link: "/developers/kailua/intro",
          },
          {
            text: "The Kailua Book",
            link: "https://boundless-xyz.github.io/kailua/",
          },
        ],
      },
    ],
  },
  {
    text: "Prove",
    collapsed: true,
    items: [
      {
        text: "Quick Start",
        link: "/provers/quick-start",
      },
      {
        text: "Core Concepts",
        items: [
          {
            text: "The Boundless Proving Stack",
            link: "/provers/proving-stack",
          },
          {
            text: "Broker Configuration & Operation",
            link: "/provers/broker",
          },
          {
            text: "Monitoring",
            link: "/provers/monitoring",
          },
          {
            text: "Performance Optimization",
            link: "/provers/performance-optimization",
          },
        ],
      },
      {
        text: "Technical Reference",
        items: [
          {
            text: "Bento Technical Design",
            link: "/provers/bento",
          },
        ],
      },
    ],
  },
  {
    text: "$ZKC",
    collapsed: true,
    items: [
      {
        text: "Quick Start",
        link: "/zkc/quick-start"
      },
      {
        text: "ZK Mining",
        link: "/zkc/mining/quick-start",
        items: [
          {
            text: "Quick Start",
            link: "/zkc/mining/quick-start",
          },
          {
            text: "Wallet Setup",
            link: "/zkc/mining/wallet-setup",
          },
          {
            text: "Mining + Claiming Rewards",
            link: "/zkc/mining/claiming-rewards",
          }
        ],
      },
      {
        text: "$ZKC as Proving Collateral",
        link: "/zkc/proving-collateral"
      },
      {
        text: "Token Source Code & Docs",
        link: "https://github.com/boundless-xyz/zkc?tab=readme-ov-file#zkc"
      },
    ],
  },
];

export function generateSitemap() {
  const allSidebarItems = [SIDEBAR_ITEMS];
  function extractRoutes(items): string[] {
    return items.flatMap((item) => {
      const routes: string[] = [];

      if (item.link) {
        routes.push(item.link);
      }

      if (item.items) {
        routes.push(...extractRoutes(item.items));
      }

      return routes;
    });
  }

  return VitePluginSitemap({
    hostname: "https://docs.boundless.network",
    dynamicRoutes: extractRoutes(allSidebarItems),
    changefreq: "weekly",
    outDir: "site/dist",
  });
}

export default defineConfig({
  logoUrl: "/logo.svg",
  topNav: [
    { text: "Explorer", link: "https://explorer.boundless.network/orders" },
    { text: "Discord", link: "https://discord.gg/aXRuD6spez" }
  ],
  font: {
    mono: {
      google: "Ubuntu Mono",
    },
  },
  vite: {
    plugins: [generateSitemap(), biomePlugin()],
  },
  sidebar: SIDEBAR_ITEMS,
  socials: [
    {
      icon: "github",
      link: "https://github.com/boundless-xyz",
    },
    {
      icon: "x",
      link: "https://x.com/boundless_xyz",
    },
  ],
  rootDir: "site",
  title: "Boundless Docs",
  theme: {
    accentColor: {
      light: "#537263", // Forest - primary accent for light mode
      dark: "#AED8C4", // Leaf - lighter accent for dark mode
    },
    variables: {
      color: {
        backgroundDark: {
          light: "#EFECE3", // Sand
          dark: "#1e1d1f",
        },
        background: {
          light: "#FFFFFF",
          dark: "#232225",
        },
      },
    },
  },
  iconUrl: {
    light: "/favicon.svg",
    dark: "/favicon.svg",
  },
  ogImageUrl: "https://docs.beboundless.xyz/og.png",
});
