void *malloc(int n);
void fill(int ****p);
int f(void) {
    int **b = malloc(8);
    if (!b) return 0;
    int **h = b;
    int ***m = &h;
    fill(&m);
    return *b != 0;
}
