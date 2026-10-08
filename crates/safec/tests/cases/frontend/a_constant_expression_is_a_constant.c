int a1[-1];
int a2[1 - 1];
int (*q)[-1];
int ok[2 * 3];

int g(int *p);

int f(int c, int *p) {
    int *r = 1 - 1;
    p = -0;
    g(1 - 1);
    r = c ? p : 1 - 1;
    return p == 1 - 1;
}
