void *malloc(int n);
void opaque(int **p);
void use3(int ***bb);
int main(void) {
    int ***bb = malloc(8);
    if (bb == 0) { return 1; }
    int **b = malloc(8);
    if (b == 0) { return 1; }
    int *x = malloc(4);
    if (x == 0) { return 1; }
    *b = x;
    *bb = b;
    opaque(&x);
    use3(bb);
    return 0;
}
