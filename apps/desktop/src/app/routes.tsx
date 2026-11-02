import type { RouteObject } from "react-router";
import { Holdings } from "../screens/Holdings";
import { Imports } from "../screens/Imports";
import { Overview } from "../screens/Overview";
import { Performance } from "../screens/Performance";
import { Assets } from "../screens/Assets";
import { Placeholder } from "../screens/Placeholder";
import { Settings } from "../screens/Settings";
import { Tax } from "../screens/Tax";
import { Layout } from "./Layout";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Layout />,
    children: [
      { index: true, element: <Overview /> },
      { path: "holdings", element: <Holdings /> },
      { path: "performance", element: <Performance /> },
      { path: "imports", element: <Imports /> },
      { path: "activity", element: <Placeholder title="Activity" /> },
      { path: "income", element: <Placeholder title="Income" /> },
      { path: "tax", element: <Tax /> },
      { path: "accounts", element: <Placeholder title="Accounts" /> },
      { path: "assets", element: <Assets /> },
      { path: "settings", element: <Settings /> },
    ],
  },
];
