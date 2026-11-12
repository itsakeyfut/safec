// An array of 100 pointers is kept whole, so `a[0] = &y;` may have missed
// `a[5]` (#918's review).
int f(void) {
    int y = 1;
    int *a[100];
    a[0] = &y;
    int *t = a[5];
    return t == 0;
}
