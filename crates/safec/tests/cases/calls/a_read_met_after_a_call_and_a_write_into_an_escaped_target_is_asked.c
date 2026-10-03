void *malloc(int n);
void keep(int **pp);
void release_all(void);

int f(void) {
    int *q;
    int **pp;
    int *a;
    int x;
    q = 0;
    pp = &q;
    keep(pp);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    return (release_all(), 0) + (*pp = a, 0) + (x = a[0]);
}
