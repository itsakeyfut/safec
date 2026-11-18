void fill(int **p);
int f(int c) {
    int *a;
    int **h = &a;
    int ***hh = &h;
    if (c) *hh = 0;
    fill(h);
    return a != 0;
}
