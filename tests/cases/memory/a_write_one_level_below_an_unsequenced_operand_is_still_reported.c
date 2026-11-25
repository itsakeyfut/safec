void h(int v, int w);
int f(int v);
int (*fp)(int);

void prefix_increment(int *q) {
    h(*q, *++q);
}

void prefix_decrement(int *q) {
    h(*q, *--q);
}

void postfix_decrement(int *q) {
    h(*q, *q--);
}

void under_a_unary_operator(int *q) {
    h(*q, -*q++);
}

void in_a_subscript_s_index(int *q) {
    h(*q, q[*q++]);
}

void in_a_subscript_s_base(int *q) {
    h(*q, (q++)[0]);
}

void in_a_condition(int *q) {
    h(*q, (q++ ? 1 : 2));
}

void in_a_then_arm(int *q, int c) {
    h(*q, (c ? *q++ : 2));
}

void in_an_else_arm(int *q, int c) {
    h(*q, (c ? 2 : *q++));
}

void under_a_conditional(int *q, int c) {
    h(*q, 1 + (c ? *q++ : 2));
}

void in_a_call_s_argument(int *q) {
    h(*q, f(*q++));
}

void in_a_callee(int *q) {
    h(*q, (q++, fp)(0));
}

void under_a_cast(int *q) {
    h(*q, (long)*q++);
}

int in_the_base_of_the_subscript_asked(int *q) {
    return (q++)[*q];
}
