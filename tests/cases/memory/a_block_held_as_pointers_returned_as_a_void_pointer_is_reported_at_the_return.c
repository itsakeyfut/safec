void *malloc(int n);
void *returned(void) {
    int **b = malloc(8);
    return b;
}
