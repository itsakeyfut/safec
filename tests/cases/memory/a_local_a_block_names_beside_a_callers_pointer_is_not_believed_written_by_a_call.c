void *malloc(int n);
void fill(int ***box, int *x);
int f(int ***in, int c) {
    int x = 0; int *a;
    if (in == 0) return 0;
    int ***box = malloc(8); if (box == 0) return 0;
    *box = *in;
    if (c) { *box = &a; }
    fill(box, &x);
    return a == 0;
}
