import type { RouteObject } from "react-router";
import { Holdings } from "../screens/Holdings";
import { Imports } from "../screens/Imports";
import { MarketData } from "../screens/MarketData";
import { Overview } from "../screens/Overview";
import { Performance } from "../screens/Performance";
import { Account } from "../screens/Account";
import { Accounts } from "../screens/Accounts";
import { Activity } from "../screens/Activity";
import { Adapters } from "../screens/Adapters";
import { Exports } from "../screens/Exports";
import { Inbox } from "../screens/Inbox";
import { Assets } from "../screens/Assets";
import { Income } from "../screens/Income";
import { Settings } from "../screens/Settings";
import { Tax } from "../screens/Tax";
import { Layout } from "./Layout";

export const routes: RouteObject[] = [
  {
    path: "/",
    element: <Layout />,
    children: [
      { index: true, element: <Overview /> },
      { path: "inbox", element: <Inbox /> },
      { path: "holdings", element: <Holdings /> },
      { path: "adapters", element: <Adapters /> },
      { path: "exports", element: <Exports /> },
      { path: "performance", element: <Performance /> },
      { path: "imports", element: <Imports /> },
      { path: "activity", element: <Activity /> },
      { path: "income", element: <Income /> },
      { path: "tax", element: <Tax /> },
      { path: "accounts", element: <Accounts /> },
      { path: "accounts/:id", element: <Account /> },
      { path: "assets", element: <Assets /> },
      { path: "market-data", element: <MarketData /> },
      { path: "settings", element: <Settings /> },
    ],
  },
];
