void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);
void stash(int **pp);
int keep(int *p);
int f(void) {
    int x;
    int r;
    int *a;
    int *b;
    int *holder;
    holder = 0;
    stash(&holder);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    b = malloc(4);
    if (b == 0) {
        return 0;
    }
    a[0] = 1;
    r = release_all() + keep(a);
    return r;
}
