void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);
void stash(int **pp);

int f(int c) {
    int x;
    int r;
    int *a;
    int *holder;
    holder = 0;
    stash(&holder);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (c ? 0 : release_all()) + (x = a[0]) + (holder = a, 0);
    return r;
}
