int f(void);
int g(void);
int h(int x);

int main(void) {
    int x = f() + g();
    return h(x);
}
