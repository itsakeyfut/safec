void *malloc(int n);
void free(void *p);
void use4(int ****bbb);
int main(void) {
    int ****c = malloc(8);
    if (c == 0) { return 1; }
    int ***bb = malloc(8);
    if (bb == 0) { return 1; }
    int **b = malloc(8);
    if (b == 0) { return 1; }
    int *x = malloc(4);
    if (x == 0) { return 1; }
    free(x);
    *b = x;
    *bb = b;
    *c = bb;
    use4(c);
    return 0;
}
