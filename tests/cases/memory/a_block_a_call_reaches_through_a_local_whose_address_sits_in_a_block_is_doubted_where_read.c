void *malloc(int n);
void fill(int ****p);
int f(int c) {
    int **b = malloc(8);
    if (!b) return 0;
    int ****box = malloc(8);
    if (!box) return 0;
    int **h = 0;
    if (c) h = b;
    *box = &h;
    fill(box);
    return *b != 0;
}
