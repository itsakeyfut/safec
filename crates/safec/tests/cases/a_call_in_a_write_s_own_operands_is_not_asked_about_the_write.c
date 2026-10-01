void *malloc(int n);
void keep(int **pp);
int release_all(void);
int now(void);

int f(int c) {
    int x;
    int r;
    int *q;
    int **pp;
    int ***ppp;
    int *a;
    q = 0;
    pp = &q;
    keep(pp);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (x = a[0]) + (*pp = a + now(), 0);
    return r;
}
