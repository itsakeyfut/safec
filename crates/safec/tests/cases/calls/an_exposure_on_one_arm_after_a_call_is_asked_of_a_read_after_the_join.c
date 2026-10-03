void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);

int f(int c) {
    int x;
    int r;
    int **hp;
    int *a;
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = release_all() + (c ? (memset(a, 0, 4) != 0) : 0) + (x = a[0]);
    return r;
}
