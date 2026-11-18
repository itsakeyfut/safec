void *malloc(int n);
void fill(int ***b);
int f(int c) {
    int *x;
    int ***box = malloc(8);
    if (!box) return 0;
    if (c) *box = &x;
    fill(box);
    return x != 0;
}
