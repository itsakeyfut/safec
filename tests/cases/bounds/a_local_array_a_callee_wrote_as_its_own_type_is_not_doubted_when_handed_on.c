void fill(int **(*p)[2]);
void use(int **p);
void run(void) {
    int **a[2];
    fill(&a);
    use(a[0]);
}
