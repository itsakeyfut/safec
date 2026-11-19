void fill(int **p);
int f(void) {
    int *a;
    fill(&a + 1);
    return a != 0;
}
