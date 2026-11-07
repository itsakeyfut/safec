void *malloc(int n);
void use(int *(*pi)[1]);
int main(void) {
    char c;
    char **b = malloc(8);
    if (b == 0) return 0;
    *b = &c;
    void *pv = b;
    use(pv);
    return 0;
}
