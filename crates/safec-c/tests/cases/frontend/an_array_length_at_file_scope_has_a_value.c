int a[2147483647 * 2];
int b[1 << 31];
int c[1 / 0];
int h[2147483647 * 2 + 1];
int u[-(1 << 31)];
int u2[+(1 / 0)];
int u3[~(1 / 0)];
int u4[!(1 / 0)];
int t[1 ? 1 / 0 : 2];
int n;
int q1[nowhere];
int d[n];
int (*e)[n];
int m2[(1 << 31) + n];
int (*fr(void))[n] {
    return 0;
}

int ok[2 * 3];
int v1[1 || 1 / 0];
int v2[0 ? 1 / 0 : 2];
int g(int m, int x[m]);

int f(int k) {
    int w[k];
    return 0;
}
