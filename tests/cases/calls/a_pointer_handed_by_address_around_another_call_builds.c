void *malloc(int n);
int use2(int **pp);
void log_it(void);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    use2(&a);
    log_it();
    return use2(&a);
}
