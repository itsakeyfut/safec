void *malloc(int n);
int f(int k) {
    int v = 0;
    int *arr[2];
    arr[0] = &v;
    int ***m = malloc(8);
    if (!m) return 0;
    *m = arr;
    int *r = **m;
    return r == 0;
}
