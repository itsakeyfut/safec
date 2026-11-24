int f(int a, ...);
int f(int a);

int g(int a, ...);
int g();

int h(int a, ...);
int h(int a, ...);

int too_few(void) {
    return h();
}

int void_past_the_parameters(void) {
    return h(1, too_few());
}

void nothing(void);

int a_void_argument(void) {
    return h(1, nothing());
}

int converted(void) {
    int (*p)(int) = h;
    return p(1);
}
