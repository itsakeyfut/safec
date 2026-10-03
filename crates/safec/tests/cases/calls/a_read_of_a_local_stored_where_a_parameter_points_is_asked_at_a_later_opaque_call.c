void *malloc(int n);
void release_all(void);

int f(int **t) {
    int x;
    int *a;
    if (t == 0) {
        return 0;
    }
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    *t = a;
    return (x = a[0]) + (release_all(), 0);
}
