void *malloc(int n);
int now(void);
void watch(int **pp);
int f(int c) {
    int *a;
    int *cursor;
    int r;
    cursor = 0;
    watch(&cursor);
    a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = 0;
    r = c ? a[0] : (cursor = a, now());
    return r;
}
