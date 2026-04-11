// Flat ESLint config for Next.js 16+.
//
// Next.js 16 removed the `next lint` subcommand in favor of running ESLint
// directly. `eslint-config-next` 16+ ships flat-config arrays from its
// `/core-web-vitals` and `/typescript` subpath exports, so we spread them
// into the top-level array.
import nextCoreWebVitals from "eslint-config-next/core-web-vitals";
import nextTypescript from "eslint-config-next/typescript";

const config = [
  {
    ignores: [
      ".next/**",
      "out/**",
      "node_modules/**",
      "lib/proto/**", // generated protobuf TS
      "next-env.d.ts",
    ],
  },
  ...nextCoreWebVitals,
  ...nextTypescript,
  {
    // The new `react-hooks` plugin v7 (pulled in by Next 16 /
    // eslint-config-next 16) ships several rules that are calibrated for
    // codebases compiled with the React Compiler. We aren't on the compiler
    // yet, so they fire on patterns that are idiomatic for pre-compiler code:
    //
    //   - `react-hooks/set-state-in-effect`: flags SSR-safe localStorage
    //     hydration and post-init async bootstraps. Both are legitimate
    //     without the compiler.
    //   - `react-hooks/preserve-manual-memoization`: only meaningful when
    //     the React Compiler is actually running over the code; otherwise
    //     it just flags useCallbacks with large capture sets as unfixable.
    //
    // Downgrade to warnings (still visible in CI output) so the lint gate
    // doesn't block on patterns that have no runtime bug.
    rules: {
      "react-hooks/set-state-in-effect": "warn",
      "react-hooks/preserve-manual-memoization": "warn",
    },
  },
];

export default config;
