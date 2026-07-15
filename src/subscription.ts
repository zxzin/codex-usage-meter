import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  getProductStatus,
  getProducts,
  purchase,
  restorePurchases,
  type Product,
} from "@choochmeque/tauri-plugin-iap-api";

export const SUBSCRIPTION_PRODUCT_ID = "com.zin.tokenmeter.monthly";

export type SubscriptionGate =
  | { status: "checking" }
  | { status: "active" | "not_required" }
  | { status: "inactive"; product: Product }
  | { status: "unavailable"; message: string };

export async function loadSubscriptionGate(): Promise<SubscriptionGate> {
  if (import.meta.env.DEV) {
    const preview = new URLSearchParams(window.location.search).get("subscription");
    if (preview === "inactive") {
      return {
        status: "inactive",
        product: {
          productId: SUBSCRIPTION_PRODUCT_ID,
          title: "Monthly Subscription",
          description: "Token Meter subscription",
          productType: "subs",
          formattedPrice: "¥1.00",
          priceCurrencyCode: "CNY",
        },
      };
    }
    if (preview === "unavailable") {
      return { status: "unavailable", message: "The subscription is temporarily unavailable." };
    }
  }

  if (!("__TAURI_INTERNALS__" in window)) {
    return { status: "not_required" };
  }

  const channel = await invoke<string>("get_release_channel");
  if (channel !== "app_store") {
    return { status: "not_required" };
  }

  const status = await getProductStatus(SUBSCRIPTION_PRODUCT_ID, "subs");
  if (status.isOwned) {
    return { status: "active" };
  }

  const { products } = await getProducts([SUBSCRIPTION_PRODUCT_ID], "subs");
  const product = products.find((candidate) => candidate.productId === SUBSCRIPTION_PRODUCT_ID);
  if (!product) {
    return {
      status: "unavailable",
      message: "The subscription is temporarily unavailable. Please try again.",
    };
  }

  return { status: "inactive", product };
}

export async function purchaseSubscription(): Promise<SubscriptionGate> {
  await purchase(SUBSCRIPTION_PRODUCT_ID, "subs");
  return loadSubscriptionGate();
}

export async function restoreSubscription(): Promise<SubscriptionGate> {
  await restorePurchases("subs");
  return loadSubscriptionGate();
}

export async function setSubscriptionWindowMode(paywall: boolean) {
  if (!("__TAURI_INTERNALS__" in window)) {
    return;
  }
  await invoke("set_subscription_window_mode", { paywall });
}

export async function openSubscriptionLink(url: string) {
  if ("__TAURI_INTERNALS__" in window) {
    await openUrl(url);
    return;
  }
  window.open(url, "_blank", "noopener,noreferrer");
}

export function subscriptionPrice(product: Product): string {
  return product.formattedPrice?.trim() || "the displayed App Store price";
}

export function isPurchaseCancellation(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return /cancel(?:led|ed)|user.?cancel/i.test(message);
}

export function subscriptionErrorMessage(error: unknown): string {
  if (isPurchaseCancellation(error)) {
    return "";
  }
  return error instanceof Error ? error.message : String(error);
}
