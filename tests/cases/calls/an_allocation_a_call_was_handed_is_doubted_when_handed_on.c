void *malloc(int n);
void init(int *p);
int use(int *p);

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    init(a);
    return use(a);
}
