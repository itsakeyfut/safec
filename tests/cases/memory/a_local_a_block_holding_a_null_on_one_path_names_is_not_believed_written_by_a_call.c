void *malloc(int n);
void fill(int ***b);
int f(int c) {
    int *a;
    int ***b = malloc(8);
    if (!b) return 0;
    *b = 0;
    if (c) *b = &a;
    fill(b);
    return a != 0;
}
