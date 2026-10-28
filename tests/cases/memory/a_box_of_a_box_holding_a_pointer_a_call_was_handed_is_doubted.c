void *malloc(int n);
void release(void *p);
void use3(int ***bb);
int main(void) {
    int ***bb = malloc(8);
    if (bb == 0) { return 1; }
    int **b = malloc(8);
    if (b == 0) { return 1; }
    int *x = malloc(4);
    if (x == 0) { return 1; }
    release(x);
    *b = x;
    *bb = b;
    use3(bb);
    return 0;
}
