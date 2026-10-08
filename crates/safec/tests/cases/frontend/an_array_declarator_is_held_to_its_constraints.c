int f(int (*q)[2][], void (*r)[2]) {
    q++;
    q[0];
    q + 1;
    r++;
    r[0];
    return 0;
}

int x[2][];
int z[0][0];
int (*fp)[3](void);
int g(void a[]);
int h(int a[0]);
int k(int *p, int a[p]);
int (*nest)(void a[]);

int m(void) {
    int w[0];
    return 0;
}

int (*ok)[2][3];
void (*gg)(void);
int u(int a[]);
int v(int n, int a[n]);
