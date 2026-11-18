void fill(int ***p);
int f(int c) {
    int *a;
    int **h;
    if (c) h = &a;
    fill(&h);
    return a != 0;
}
