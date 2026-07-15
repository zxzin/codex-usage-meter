declare module "@choochmeque/tauri-plugin-iap-api" {
  export interface PricingPhase {
    formattedPrice: string;
    priceCurrencyCode: string;
    billingPeriod: string;
    billingCycleCount: number;
    recurrenceMode: number;
  }

  export interface SubscriptionOffer {
    offerId?: string;
    pricingPhases: PricingPhase[];
  }

  export interface Product {
    productId: string;
    title: string;
    description: string;
    productType: string;
    formattedPrice?: string;
    priceCurrencyCode?: string;
    subscriptionOfferDetails?: SubscriptionOffer[];
  }

  export interface ProductStatus {
    productId: string;
    isOwned: boolean;
    expirationTime?: number;
    isAutoRenewing?: boolean;
  }

  export function getProducts(
    productIds: string[],
    productType?: "subs" | "inapp",
  ): Promise<{ products: Product[] }>;

  export function getProductStatus(
    productId: string,
    productType?: "subs" | "inapp",
  ): Promise<ProductStatus>;

  export function purchase(
    productId: string,
    productType?: "subs" | "inapp",
  ): Promise<unknown>;

  export function restorePurchases(
    productType?: "subs" | "inapp",
  ): Promise<unknown>;
}
