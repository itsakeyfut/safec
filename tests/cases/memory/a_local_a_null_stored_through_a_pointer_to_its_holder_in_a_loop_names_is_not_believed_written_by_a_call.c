void fill(int **p);
int f(int n) {
    int *a;
    int **h = &a;
    int ***hh = &h;
    for (int i = 0; i < n; i++) *hh = 0;
    fill(h);
    return a != 0;
}
