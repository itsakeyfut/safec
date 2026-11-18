void *malloc(int n);
void fillb(int ***b);
int f(int c) {
    int *a;
    int **h = &a;
    int ***hh = &h;
    if (c) *hh = 0;
    int ***b = malloc(8);
    if (!b) return 0;
    *b = h;
    fillb(b);
    return a != 0;
}
